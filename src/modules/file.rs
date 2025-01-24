use anyhow::{Context, Result};
use std::{collections::HashMap, fs, os::windows::fs::MetadataExt};

pub fn calc_size(path: &str) -> Result<u64> {
    let metadata = fs::metadata(path).context("ファイルのメタデータを取得できませんでした.")?;
    Ok(metadata.file_size())
}

// pub fn calc_crf_size() -> Result<HashMap<u32, u64>> {
//     let dir = fs::read_dir("./out").context("フォルダーを開けませんでした.")?;
//     let mut size_map: HashMap<u32, u64> = HashMap::new();

//     for entry in dir {
//         let entry = entry?;
//         let metadata = entry.metadata()?;
//         let file_name = entry.file_name();
//         let size = metadata.file_size();

//         let crf = file_name.to_str().and_then(|name| {
//             name.split("--crf-")
//                 .nth(1)
//                 .and_then(|s| s.split('.').next())
//                 .and_then(|s| s.parse::<u32>().ok())
//         });

//         if let Some(crf) = crf {
//             size_map.insert(crf, size);
//         }
//     }

//     Ok(size_map)
// }
