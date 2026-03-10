use crate::config::ConfigManager;
use anyhow::Result;

pub async fn show_instance(config_mgr: &ConfigManager, name: String) -> Result<()> {
    let metadata = config_mgr.get_instance(&name)?;
    println!("Instance: {}", metadata.name);
    println!("Version: {}", metadata.version);
    println!("Port: {}", metadata.port);
    println!("Connection string: postgresql://postgres@localhost:{}/postgres", metadata.port);
    println!("Data directory: {}", config_mgr.get_instance_dir(&name).join("data").display());
    Ok(())
}
