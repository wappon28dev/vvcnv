use anyhow::{anyhow, bail, Context, Result};
use ffmpeg_sidecar::{
    command::FfmpegCommand,
    event::{
        AudioStream, FfmpegDuration, FfmpegEvent, FfmpegProgress, LogLevel, StreamTypeSpecificData,
        VideoStream,
    },
};
use indicatif::ProgressBar;
use itertools::iproduct;
use std::{ffi::OsStr, io, iter, ops, time::Duration};

use super::file;

#[derive(Debug, Clone)]
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

    pub fn to_file_name(&self) -> String {
        let (w, h) = self.to_wh();

        format!("{}x{}", w, h)
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

#[derive(Debug, Clone)]
pub struct VideoStat {
    pub path: String,
    pub video_stream: VideoStream,
    pub audio_streams: Vec<AudioStream>,
    pub duration: Duration,
    pub file_size: u64,
}

#[derive(Debug)]
pub enum VideoStatErr {
    NoVideoStreamFound,
    MultipleVideoStreamFound,
    NoDurationFound,
    FfmpegError(anyhow::Error),
    FileError(io::Error),
}

#[derive(Debug, Clone)]
pub struct VideoConfigParams {
    pub res: VideoRes,
    pub fps: u32,
    pub crf: u32,
}

pub struct VideoConfigParamsIter {
    res: Vec<VideoRes>,
    fps: Vec<u32>,
    crf: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct VideoConfig {
    pub res: VideoRes,
    pub fps: u32,
    pub crf: u32,
    pub has_audio: bool,
}

impl VideoConfig {
    pub fn from_stat(stat: VideoStat) -> Self {
        let VideoStat {
            video_stream,
            audio_streams,
            ..
        } = stat;

        let res = VideoRes::from_wh(video_stream.width as i32, video_stream.height as i32);
        let fps = video_stream.fps as u32;
        // ref: https://trac.ffmpeg.org/wiki/Encode/H.264#:~:text=23%20is%20the%20default
        let crf = 23;
        let has_audio = !audio_streams.is_empty();

        Self {
            res,
            fps,
            crf,
            has_audio,
        }
    }

    pub fn to_file_name(&self) -> String {
        format!(
            "--crf-{}--fps-{}--res-{}",
            self.crf,
            self.fps,
            self.res.to_file_name()
        )
    }
}

pub struct VideoProcessParams {
    pub output_path: String,
    pub config: VideoConfig,
}

pub fn handle_ffmpeg_event_log(
    level: LogLevel,
    err: String,
    ignore_no_input: bool,
) -> Result<(), anyhow::Error> {
    let err_clone = err.clone();
    match level {
        LogLevel::Fatal | LogLevel::Error => {
            let err_body = err_clone.split("[fatal] ").last().unwrap();
            let expected = err_body == "At least one output file must be specified";

            if expected && !ignore_no_input {
                return Err(anyhow!("入力ファイルが指定されていません"));
            }

            Err(anyhow!(err_body))
        }

        LogLevel::Warning => Ok(()).inspect(|_| {
            println!("警告: {}", err);
        }),
        _ => Ok(()),
    }
}

pub async fn stat(input_path: String) -> Result<VideoStat, VideoStatErr> {
    let mut runner = FfmpegCommand::new()
        .input(input_path.clone())
        .spawn()
        .unwrap();

    let mut input_duration_sec: Option<f64> = None;
    let mut input_streams: Vec<StreamTypeSpecificData> = Vec::new();

    for e in runner.iter().unwrap() {
        match e {
            FfmpegEvent::ParsedDuration(FfmpegDuration { duration, .. }) => {
                input_duration_sec = Some(duration);
            }
            FfmpegEvent::ParsedInputStream(s) => {
                input_streams.push(s.type_specific_data);
            }
            FfmpegEvent::Log(level, err) => {
                handle_ffmpeg_event_log(level, err, true).map_err(VideoStatErr::FfmpegError)?;
            }
            _ => {
                // println!("{:?}", e);
            }
        }
    }

    let mut video_streams = input_streams.iter().filter_map(|s| match s {
        StreamTypeSpecificData::Video(v) => Some(v),
        _ => None,
    });

    let video_stream = match video_streams.clone().count() {
        0 => Err(VideoStatErr::NoVideoStreamFound),
        1 => Ok(video_streams.next().unwrap().clone()),
        _ => Err(VideoStatErr::MultipleVideoStreamFound),
    }?;

    let audio_streams = input_streams
        .iter()
        .filter_map(|a| match a {
            StreamTypeSpecificData::Audio(a) => Some(a.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();

    let duration_sec = input_duration_sec.ok_or(VideoStatErr::NoDurationFound)?;
    let duration = Duration::from_secs_f64(duration_sec);

    let file_size = file::calc_size(&input_path).map_err(VideoStatErr::FileError)?;

    Ok(VideoStat {
        path: input_path,
        video_stream,
        audio_streams,
        duration,
        file_size,
    })
}

pub async fn process(stat: VideoStat, params: VideoProcessParams, pb: ProgressBar) -> Result<()> {
    let VideoProcessParams {
        output_path,
        config,
    } = params;

    let arg = config.res.to_args().map_err(|e| {
        match e {
            ToStrError::BothAreDynamicValue => {
                anyhow!("高さと幅のどちらも動的な値 (`-1`) に設定されています. どちらか一方を固定値にしてください.")
            },
        }
    }).context("映像の高さと幅の指定に失敗しました.")?;

    let arg_os_str: Vec<&OsStr> = arg.split_whitespace().map(OsStr::new).collect();

    let mut runner = FfmpegCommand::new()
        .input(stat.path)
        .crf(config.crf)
        .args(arg_os_str)
        .output(output_path)
        .overwrite()
        .spawn()
        .unwrap();

    for e in runner.iter().unwrap() {
        match e {
            FfmpegEvent::Progress(FfmpegProgress {
                frame: current_frame,
                ..
            }) => {
                let total_frame = (stat.duration.as_secs() as f32) * stat.video_stream.fps;
                pb.set_length(total_frame as u64);
                pb.set_position(current_frame as u64);
                pb.set_message("エンコード中...");
            }
            FfmpegEvent::Log(level, err) => {
                if let Err(e) = handle_ffmpeg_event_log(level, err, false) {
                    println!("here!: {:?}", e);
                    return Err(anyhow!(e));
                }
            }
            _ => {
                // println!("{:?}", e);
            }
        }
    }

    Ok(())
}
