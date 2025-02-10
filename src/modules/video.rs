use std::ffi::OsStr;

use anyhow::{anyhow, bail, Context, Result};
use ffmpeg_sidecar::{
    command::FfmpegCommand,
    event::{FfmpegDuration, FfmpegEvent, FfmpegProgress, LogLevel, StreamTypeSpecificData},
};
use humansize::ToF64;
use indicatif::ProgressBar;

pub enum VideoRes {
    R240p,
    R360p,
    R480p,
    R720p,
    R1080p,
    R1440p,
    R2160p,
    R4320p,
    Other(i32, i32),
}

enum ToStrError {
    BothAreDynamicValue,
}

impl VideoRes {
    pub fn to_wh(&self) -> (i32, i32) {
        match self {
            VideoRes::R240p => (426, 240),
            VideoRes::R360p => (640, 360),
            VideoRes::R480p => (854, 480),
            VideoRes::R720p => (1280, 720),
            VideoRes::R1080p => (1920, 1080),
            VideoRes::R1440p => (2560, 1440),
            VideoRes::R2160p => (3840, 2160),
            VideoRes::R4320p => (7680, 4320),
            VideoRes::Other(w, h) => (*w, *h),
        }
    }

    pub fn to_name(&self) -> &str {
        match self {
            VideoRes::R240p => "240p (SD)",
            VideoRes::R360p => "360p (SD)",
            VideoRes::R480p => "480p (SD)",
            VideoRes::R720p => "720p (HD)",
            VideoRes::R1080p => "1080p (FHD)",
            VideoRes::R1440p => "1440p (QHD)",
            VideoRes::R2160p => "2160p (4K)",
            VideoRes::R4320p => "4320p (8K)",
            VideoRes::Other(_, _) => "その他",
        }
    }

    pub fn from_wh(width: i32, height: i32) -> Self {
        match (width, height) {
            (426, 240) => VideoRes::R240p,
            (640, 360) => VideoRes::R360p,
            (854, 480) => VideoRes::R480p,
            (1280, 720) => VideoRes::R720p,
            (1920, 1080) => VideoRes::R1080p,
            (2560, 1440) => VideoRes::R1440p,
            (3840, 2160) => VideoRes::R2160p,
            (7680, 4320) => VideoRes::R4320p,
            _ => VideoRes::Other(width, height),
        }
    }

    pub fn to_args(&self) -> Result<String, ToStrError> {
        let (width, height) = self.to_wh();

        match (width, height) {
            (-1, -1) => Err(ToStrError::BothAreDynamicValue),
            (_, -1) | (-1, _) => Ok(format!("-vf scale={}:{}", width, height)),
            _ => Ok(format!("-s {}x{}", width, height)),
        }
    }
}

pub struct VideoConfig {
    pub res: Option<VideoRes>,
    pub fps: Option<f64>,
    pub crf: u32,
    pub has_audio: bool,
}

pub struct VideoProcessParams {
    pub input_path: String,
    pub output_path: String,
    pub video_config: VideoConfig,
}

pub async fn process(params: VideoProcessParams, pb: ProgressBar) -> Result<()> {
    let VideoProcessParams {
        input_path,
        output_path,
        video_config,
    } = params;

    let arg = match video_config.res {
        Some(res) => res.to_args().map_err(|e| {
            match e {
                ToStrError::BothAreDynamicValue => {
                    anyhow!("高さと幅のどちらも動的な値 (`-1`) に設定されています. どちらか一方を固定値にしてください.")
                },
            }
        }).context("映像の高さと幅の指定に失敗しました.")?,
        None => "".to_string(),
    };
    let arg_os_str: Vec<&OsStr> = arg.split_whitespace().map(OsStr::new).collect();

    let mut runner = FfmpegCommand::new()
        .input(input_path)
        .crf(video_config.crf)
        .args(arg_os_str)
        .output(output_path)
        .overwrite()
        .spawn()
        .unwrap();

    let mut input_duration_sec: Option<f64> = None;
    let mut input_streams: Vec<StreamTypeSpecificData> = Vec::new();

    for e in runner.iter().unwrap() {
        let mut video_streams = input_streams.iter().filter_map(|s| match s {
            StreamTypeSpecificData::Video(v) => Some(v),
            _ => None,
        });
        let video_stream = video_streams.next();

        match e {
            FfmpegEvent::ParsedDuration(FfmpegDuration { duration, .. }) => {
                input_duration_sec = Some(duration);
            }
            FfmpegEvent::ParsedInputStream(s) => {
                input_streams.push(s.type_specific_data);
            }
            FfmpegEvent::Progress(FfmpegProgress {
                frame: current_frame,
                ..
            }) => {
                if input_duration_sec.is_some() && video_stream.is_some() {
                    let total_frame =
                        input_duration_sec.unwrap() * video_stream.unwrap().fps.to_f64();
                    pb.set_length(total_frame as u64);
                    pb.set_position(current_frame as u64);
                    pb.set_message("エンコード中...");
                }
            }
            FfmpegEvent::Log(level, err) => match level {
                LogLevel::Fatal | LogLevel::Error => {
                    bail!(
                        "FFmpeg エラーが発生しました: {}",
                        err.split("[fatal]").last().unwrap()
                    );
                }
                LogLevel::Warning => {
                    println!("警告: {}", err);
                }
                _ => {}
            },
            _ => {
                // println!("{:?}", e);
            }
        }
    }

    Ok(())
}
