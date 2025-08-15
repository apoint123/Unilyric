use std::path::PathBuf;

use directories::ProjectDirs;

pub fn get_app_data_dir() -> Option<PathBuf> {
    if let Some(proj_dirs) = ProjectDirs::from("com", "Unilyric", "Unilyric") {
        let data_dir = proj_dirs.data_local_dir();
        if !data_dir.exists()
            && let Err(e) = std::fs::create_dir_all(data_dir)
        {
            tracing::error!("[UniLyric] 无法创建应用数据目录 {data_dir:?}: {e}");
            return None;
        }
        Some(data_dir.to_path_buf())
    } else {
        tracing::error!("[UniLyric] 无法获取应用数据目录。");
        None
    }
}
