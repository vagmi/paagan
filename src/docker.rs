use anyhow::{Context, Result};
use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, RemoveContainerOptions,
    StartContainerOptions, WaitContainerOptions,
};
use bollard::models::HostConfig;
use std::collections::HashMap;

use crate::config::InitMode;

pub struct DockerManager {
    docker: Docker,
}

pub struct ContainerSpec<'a> {
    pub name: &'a str,
    pub image: &'a str,
    pub version: &'a str,
    pub init_mode: InitMode,
    pub port: u16,
    pub data_dir: &'a str,
    pub archive_dir: &'a str,
    pub backup_dir: &'a str,
    pub restore_dir: Option<&'a str>,
    pub shared_preload_libraries: Option<&'a str>,
}

const CNPG_DATA_MOUNT: &str = "/var/lib/postgresql/data";
const CNPG_PGDATA: &str = "/var/lib/postgresql/data/pgdata";
const CNPG_PGDATA_SUBDIR: &str = "pgdata";


impl DockerManager {
    pub fn new() -> Result<Self> {
        let docker = Docker::connect_with_local_defaults().context("Failed to connect to Docker")?;
        Ok(Self { docker })
    }

    pub fn mount_info(
        &self,
        init_mode: InitMode,
        version: &str,
    ) -> (&'static str, Option<String>) {
        match init_mode {
            InitMode::Standard => self.standard_mount_info(version),
            InitMode::Cnpg => (CNPG_DATA_MOUNT, Some(CNPG_PGDATA_SUBDIR.to_string())),
        }
    }

    fn standard_mount_info(&self, version: &str) -> (&'static str, Option<String>) {
        let major_str = version.split('.').next().unwrap_or("");
        let major_prefix: String = major_str
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let major = major_prefix.parse::<u32>().unwrap_or(0);

        if major >= 18 || version == "latest" {
            let v = if version == "latest" {
                "18"
            } else {
                &major_prefix
            };
            ("/var/lib/postgresql", Some(format!("{}/docker", v)))
        } else {
            ("/var/lib/postgresql/data", None)
        }
    }

    async fn pull_image(&self, image: &str) -> Result<()> {
        eprintln!("Pulling image {}...", image);
        use futures_util::StreamExt;
        let mut pull_stream = self.docker.create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: image.to_string(),
                ..Default::default()
            }),
            None,
            None,
        );
        while let Some(pull_result) = pull_stream.next().await {
            let _ = pull_result.context("Failed to pull image")?;
        }
        Ok(())
    }

    async fn host_user_str() -> Result<String> {
        let uid_output = tokio::process::Command::new("id").arg("-u").output().await?;
        let gid_output = tokio::process::Command::new("id").arg("-g").output().await?;
        let user_id = String::from_utf8_lossy(&uid_output.stdout).trim().to_string();
        let group_id = String::from_utf8_lossy(&gid_output.stdout).trim().to_string();
        Ok(format!("{}:{}", user_id, group_id))
    }

    /// Run the one-shot CNPG-style init container: ensures `$PGDATA` is
    /// initdb'd. Idempotent — if `$PGDATA/PG_VERSION` already exists the body
    /// is skipped.
    ///
    /// Runs as host UID:GID so writes against the bind mount work on Colima
    /// (sshfs ignores chown and remaps owner to the configured user). Since
    /// the host UID typically isn't in the image's `/etc/passwd`, we bind a
    /// synthetic passwd/group file in so `initdb`'s `getpwuid` succeeds.
    pub async fn run_init_container(&self, spec: &ContainerSpec<'_>) -> Result<()> {
        debug_assert!(matches!(spec.init_mode, InitMode::Cnpg));

        // Always re-asserts the superuser password via a temporary local-only
        // postgres start. Belt-and-suspenders for custom images where initdb's
        // --pwfile may be skipped (PG_VERSION already present) or where the
        // image pre-populated the data dir without a known password.
        let init_script = r#"
mkdir -p "$PGDATA"
if [ ! -s "$PGDATA/PG_VERSION" ]; then
    PWFILE=$(mktemp)
    printf '%s' "$POSTGRES_PASSWORD" > "$PWFILE"
    initdb \
        -D "$PGDATA" -U postgres \
        --auth-host=scram-sha-256 --auth-local=trust \
        --pwfile="$PWFILE"
    rm -f "$PWFILE"
    {
        echo "host all all 0.0.0.0/0 scram-sha-256"
        echo "host all all ::/0      scram-sha-256"
    } >> "$PGDATA/pg_hba.conf"
fi

pg_ctl -D "$PGDATA" \
    -o "-c listen_addresses='' -c unix_socket_directories=/tmp" \
    -w start
psql -h /tmp -U postgres -v ON_ERROR_STOP=1 \
    -c "ALTER USER postgres WITH PASSWORD '$POSTGRES_PASSWORD';"
pg_ctl -D "$PGDATA" -m fast -w stop
"#;

        let user_str = Self::host_user_str().await?;
        let (passwd_path, group_path) = ensure_passwd_files(spec.data_dir, &user_str).await?;

        let binds = vec![
            format!("{}:{}", spec.data_dir, CNPG_DATA_MOUNT),
            format!("{}:/etc/passwd:ro", passwd_path),
            format!("{}:/etc/group:ro", group_path),
        ];
        let env = vec![
            format!("PGDATA={}", CNPG_PGDATA),
            "POSTGRES_PASSWORD=postgres".to_string(),
        ];

        let config = Config {
            image: Some(spec.image.to_string()),
            user: Some(user_str),
            env: Some(env),
            entrypoint: Some(vec![
                "bash".to_string(),
                "-eu".to_string(),
                "-c".to_string(),
            ]),
            cmd: Some(vec![init_script.to_string()]),
            host_config: Some(HostConfig {
                binds: Some(binds),
                ..Default::default()
            }),
            ..Default::default()
        };

        let init_name = format!("paagan-{}-init", spec.name);

        // Best-effort cleanup of any stale init container from a prior run.
        let _ = self
            .docker
            .remove_container(
                &init_name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        let options = CreateContainerOptions {
            name: init_name.clone(),
            ..Default::default()
        };

        let response = self.docker.create_container(Some(options), config).await?;
        self.docker
            .start_container(&response.id, None::<StartContainerOptions<String>>)
            .await?;

        use futures_util::StreamExt;
        let mut wait_stream = self
            .docker
            .wait_container(&response.id, None::<WaitContainerOptions<String>>);
        let mut exit_code: i64 = 0;
        while let Some(result) = wait_stream.next().await {
            match result {
                Ok(r) => exit_code = r.status_code,
                Err(e) => return Err(e).context("init container wait failed"),
            }
        }

        let _ = self
            .docker
            .remove_container(
                &response.id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        if exit_code != 0 {
            anyhow::bail!("init container exited with status {}", exit_code);
        }
        Ok(())
    }

    pub async fn create_instance_container(&self, spec: &ContainerSpec<'_>) -> Result<String> {
        self.pull_image(spec.image).await?;

        if matches!(spec.init_mode, InitMode::Cnpg) {
            self.run_init_container(spec).await?;
        }

        let mut port_bindings = HashMap::new();
        port_bindings.insert(
            "5432/tcp".to_string(),
            Some(vec![bollard::models::PortBinding {
                host_ip: Some("0.0.0.0".to_string()),
                host_port: Some(spec.port.to_string()),
            }]),
        );

        // Explicitly expose 5432 so the binding works even for custom images
        // (e.g. cnpg-style) whose Dockerfile may not declare EXPOSE 5432.
        let mut exposed_ports: HashMap<String, HashMap<(), ()>> = HashMap::new();
        exposed_ports.insert("5432/tcp".to_string(), HashMap::new());

        let (mount_point, _) = self.mount_info(spec.init_mode, spec.version);
        let mut binds = vec![
            format!("{}:{}", spec.data_dir, mount_point),
            format!("{}:/archive", spec.archive_dir),
            format!("{}:/backups", spec.backup_dir),
        ];
        if let Some(rd) = spec.restore_dir {
            binds.push(format!("{}:/restore", rd));
        }
        if matches!(spec.init_mode, InitMode::Cnpg) {
            let user_str = Self::host_user_str().await?;
            let (passwd_path, group_path) =
                ensure_passwd_files(spec.data_dir, &user_str).await?;
            binds.push(format!("{}:/etc/passwd:ro", passwd_path));
            binds.push(format!("{}:/etc/group:ro", group_path));
        }

        let host_config = HostConfig {
            port_bindings: Some(port_bindings),
            binds: Some(binds),
            ..Default::default()
        };

        let config = match spec.init_mode {
            InitMode::Standard => {
                let user_str = Self::host_user_str().await?;
                Config {
                    image: Some(spec.image.to_string()),
                    user: Some(user_str),
                    env: Some(vec![
                        "POSTGRES_PASSWORD=password".to_string(),
                        "POSTGRES_HOST_AUTH_METHOD=trust".to_string(),
                    ]),
                    exposed_ports: Some(exposed_ports),
                    host_config: Some(host_config),
                    cmd: Some(self.standard_postgres_cmd(spec.shared_preload_libraries)),
                    ..Default::default()
                }
            }
            InitMode::Cnpg => {
                let user_str = Self::host_user_str().await?;
                Config {
                    image: Some(spec.image.to_string()),
                    user: Some(user_str),
                    env: Some(vec![format!("PGDATA={}", CNPG_PGDATA)]),
                    exposed_ports: Some(exposed_ports),
                    host_config: Some(host_config),
                    entrypoint: Some(vec!["postgres".to_string()]),
                    cmd: Some(self.cnpg_postgres_cmd(spec.shared_preload_libraries)),
                    ..Default::default()
                }
            }
        };

        let options = CreateContainerOptions {
            name: format!("paagan-{}", spec.name),
            ..Default::default()
        };

        let response = self.docker.create_container(Some(options), config).await?;
        self.docker
            .start_container(&response.id, None::<StartContainerOptions<String>>)
            .await?;

        Ok(response.id)
    }

    fn standard_postgres_cmd(&self, shared_preload_libraries: Option<&str>) -> Vec<String> {
        let mut cmd = vec![
            "postgres".to_string(),
            "-c".to_string(),
            "wal_level=replica".to_string(),
            "-c".to_string(),
            "archive_mode=on".to_string(),
            "-c".to_string(),
            "archive_command=cp %p /archive/%f".to_string(),
        ];
        if let Some(s) = shared_preload_libraries {
            cmd.push("-c".to_string());
            cmd.push(format!("shared_preload_libraries={}", s));
        }
        cmd
    }

    fn cnpg_postgres_cmd(&self, shared_preload_libraries: Option<&str>) -> Vec<String> {
        let mut cmd = vec![
            "-D".to_string(),
            CNPG_PGDATA.to_string(),
            "-c".to_string(),
            "listen_addresses=*".to_string(),
            "-c".to_string(),
            "unix_socket_directories=/tmp".to_string(),
            "-c".to_string(),
            "wal_level=replica".to_string(),
            "-c".to_string(),
            "archive_mode=on".to_string(),
            "-c".to_string(),
            "archive_command=cp %p /archive/%f".to_string(),
        ];
        if let Some(s) = shared_preload_libraries {
            cmd.push("-c".to_string());
            cmd.push(format!("shared_preload_libraries={}", s));
        }
        cmd
    }

    pub async fn list_instances(&self) -> Result<Vec<(String, String)>> {
        let mut filters = HashMap::new();
        filters.insert("name".to_string(), vec!["paagan-".to_string()]);

        let options = ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        };

        let containers = self.docker.list_containers(Some(options)).await?;
        let mut result = Vec::new();
        for container in containers {
            if let Some(names) = container.names {
                for name in names {
                    if let Some(instance_name) = name.strip_prefix("/paagan-") {
                        if instance_name.ends_with("-init") {
                            continue;
                        }
                        result.push((
                            instance_name.to_string(),
                            container.state.clone().unwrap_or_default(),
                        ));
                    }
                }
            }
        }
        Ok(result)
    }

    pub async fn start_container(&self, name: &str) -> Result<()> {
        let container_name = format!("paagan-{}", name);
        self.docker
            .start_container(&container_name, None::<StartContainerOptions<String>>)
            .await?;
        Ok(())
    }

    pub async fn stop_container(&self, name: &str) -> Result<()> {
        let container_name = format!("paagan-{}", name);
        self.docker.stop_container(&container_name, None).await?;
        Ok(())
    }

    pub async fn delete_container(&self, name: &str) -> Result<()> {
        let container_name = format!("paagan-{}", name);
        let options = bollard::container::RemoveContainerOptions {
            force: true,
            ..Default::default()
        };
        self.docker
            .remove_container(&container_name, Some(options))
            .await?;
        Ok(())
    }

    pub async fn run_basebackup(
        &self,
        name: &str,
        backup_path: &str,
        init_mode: InitMode,
    ) -> Result<()> {
        let container_name = format!("paagan-{}", name);
        let mut cmd = tokio::process::Command::new("docker");
        cmd.arg("exec").arg(&container_name).arg("pg_basebackup");
        // Cnpg auth uses scram on TCP and trust on local socket; force the
        // local socket so the backup can run without a password.
        if matches!(init_mode, InitMode::Cnpg) {
            cmd.arg("-h").arg("/tmp");
        }
        cmd.arg("-U")
            .arg("postgres")
            .arg("-D")
            .arg(backup_path)
            .arg("--checkpoint=fast")
            .arg("-Ft")
            .arg("-z")
            .arg("-X")
            .arg("stream")
            .arg("-P");

        let status = cmd.status().await.context("Failed to run pg_basebackup")?;

        if !status.success() {
            anyhow::bail!("pg_basebackup failed");
        }
        Ok(())
    }

    pub async fn run_wal_switch(&self, name: &str, init_mode: InitMode) -> Result<()> {
        let container_name = format!("paagan-{}", name);
        let mut cmd = tokio::process::Command::new("docker");
        cmd.arg("exec").arg(&container_name).arg("psql");
        if matches!(init_mode, InitMode::Cnpg) {
            cmd.arg("-h").arg("/tmp");
        }
        cmd.arg("-U")
            .arg("postgres")
            .arg("-c")
            .arg("SELECT pg_switch_wal();");

        let status = cmd.status().await.context("Failed to run pg_switch_wal")?;

        if !status.success() {
            anyhow::bail!("pg_switch_wal failed");
        }
        Ok(())
    }

    pub async fn exec_psql(&self, name: &str, init_mode: InitMode) -> Result<()> {
        let container_name = format!("paagan-{}", name);
        let mut cmd = tokio::process::Command::new("docker");
        cmd.arg("exec").arg("-it").arg(&container_name).arg("psql");
        if matches!(init_mode, InitMode::Cnpg) {
            cmd.arg("-h").arg("/tmp");
        }
        cmd.arg("-U").arg("postgres");

        let status = cmd.status().await.context("Failed to execute docker exec")?;

        if !status.success() {
            anyhow::bail!("psql command failed");
        }
        Ok(())
    }
}

/// Write `<data_dir>/.paagan-passwd` and `.paagan-group` containing the
/// minimum entries needed for `initdb` and `postgres` to look up the host
/// UID/GID by name. We bind these in as `/etc/passwd` and `/etc/group` for
/// cnpg-mode containers, since the underlying image generally doesn't have
/// the host UID in its passwd file and we can't `chown` across Colima sshfs.
async fn ensure_passwd_files(data_dir: &str, user_str: &str) -> Result<(String, String)> {
    use tokio::fs;

    let parts: Vec<&str> = user_str.split(':').collect();
    let uid = parts.first().copied().unwrap_or("0");
    let gid = parts.get(1).copied().unwrap_or("0");

    let parent = std::path::Path::new(data_dir)
        .parent()
        .context("data_dir has no parent")?;
    let passwd_path = parent.join(".paagan-passwd");
    let group_path = parent.join(".paagan-group");

    let passwd = format!(
        "root:x:0:0:root:/root:/bin/bash\n\
         postgres:x:{uid}:{gid}:PostgreSQL:/var/lib/postgresql:/bin/bash\n\
         nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin\n"
    );
    let group = format!(
        "root:x:0:\n\
         postgres:x:{gid}:\n\
         nobody:x:65534:\n"
    );

    fs::write(&passwd_path, passwd).await?;
    fs::write(&group_path, group).await?;

    Ok((
        passwd_path.to_string_lossy().into_owned(),
        group_path.to_string_lossy().into_owned(),
    ))
}
