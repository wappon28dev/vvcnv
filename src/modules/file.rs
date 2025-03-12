use anyhow::{Context, Result};
use std::{fs, io};

pub fn calc_size(path: &str) -> Result<u64, io::Error> {
    let metadata = fs::metadata(path)?;
    Ok(metadata.len())
}

pub fn get_file_name(path: &str) -> (String, String) {
    let file_name = path.split('/').last().unwrap();
    let mut file_name_parts = file_name.split('.');
    let file_name_without_ext = file_name_parts.next().unwrap();
    let ext = file_name_parts.next().unwrap();
    (file_name_without_ext.to_string(), ext.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_get_file_name() {
        let path = "assets/2.mp4";
        let (file_name, file_name_without_ext) = super::get_file_name(path);
        assert_eq!(file_name, "2");
        assert_eq!(file_name_without_ext, "mp4");
    }
}
