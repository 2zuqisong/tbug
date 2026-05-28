use std::path::PathBuf;

/// Default tbug configuration directory name, placed under the user's home.
pub const TBUG_DIR_NAME: &str = ".tbug";

/// Returns the path to `$HOME/.tbug`, creating the directory if it doesn't exist.
pub fn get_tbug_home() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let path = PathBuf::from(home).join(TBUG_DIR_NAME);
    if !path.exists() {
        let _ = std::fs::create_dir_all(&path);
    }
    path
}

/// Bootstrap tbug: create the home directory and report status.
pub fn init() {
    let path = get_tbug_home();
    println!("tbug home directory: {}", path.display());
    println!("tbug initialized successfully.");
}
