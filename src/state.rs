use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct AppState {
    pub db_path: PathBuf,
}
