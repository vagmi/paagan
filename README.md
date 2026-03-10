# 🐘 Paagan 

`paagan` is a lightweight CLI tool to manage multiple PostgreSQL versions and instances locally using Docker. 

The primary motivation for this project is to have a developer experience similar to **CloudNativePG (CNPG)** but running locally on a development machine. It simplifies tasks like running specific PostgreSQL versions, managing multiple isolated databases, and performing **Point-In-Time Recovery (PITR)** through forking.

## Features

- **Multi-Version Support:** Run any PostgreSQL version (including 15, 16, 17, and the new 18+ structure).
- **Isolation:** Each instance gets its own data, WAL archive, and backup directories.
- **Dynamic Port Management:** Automatically assigns and remembers unused ports for each instance.
- **Local PITR:** Fork an existing database to a new instance at a specific point in time.
- **Cloud-Native Logic:** Mimics production-grade database management patterns (WAL archiving, base backups).
- **Portable:** Automatically handles UID/GID mapping to ensure Docker has correct permissions on your local filesystem.

## Installation

Ensure you have Rust and Docker installed.

```bash
cargo build --release
cp target/release/paagan /usr/local/bin/ # or wherever you want it
```

*Note: Ensure your `DOCKER_HOST` is correctly set if you are using Colima or a non-standard Docker socket.*

## Directory Structure

All configuration and data are stored in `~/.paagan`:
- `instances.json`: Metadata for all managed instances.
- `instances/<name>/data`: PostgreSQL data directory.
- `instances/<name>/archive`: WAL archive directory.
- `instances/<name>/backups`: Base backups used for forking.

## Commands

### `list`
Lists all database containers managed by `paagan` along with their versions, ports, and current Docker status.
```bash
paagan list
```

### `create`
Creates a new PostgreSQL instance. It pulls the required image, sets up the directory structure, and starts the container.
```bash
paagan create --version 18-alpine my-db
```

### `start` / `stop`
Starts or stops an existing database instance. `start` will recreate the container if it was manually removed, ensuring your data is always accessible.
```bash
paagan stop my-db
paagan start my-db
```

### `psql`
Opens an interactive `psql` session to the specified instance.
```bash
paagan psql my-db
```

### `show`
Shows detailed information about an instance, including the connection string and data directory paths.
```bash
paagan show my-db
```

### `fork`
Creates a new instance from an existing one. If `--at` is provided, it performs a **Point-In-Time Recovery (PITR)**.
```bash
# Direct fork
paagan fork source-db forked-db

# Fork to a specific timestamp (PITR)
paagan fork --at "2026-03-10 14:30:00+00" source-db recovery-db
```

### `delete`
Removes the database instance, its Docker container, and all associated data on disk.
```bash
paagan delete my-db
```

## How Forking Works (The CNPG way)
1. `paagan` triggers a `pg_basebackup` on the source instance.
2. It ensures all pending WAL logs are archived.
3. It extracts the backup into the new instance's data directory.
4. It configures a `restore_command` to pull logs from the source's archive.
5. If a timestamp is provided, it sets `recovery_target_time`.
6. The new instance starts in recovery mode, reaches the target, promotes itself, and becomes a standalone database.
