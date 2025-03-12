use anyhow::{anyhow, bail, Context, Result};
use core::fmt;
use ffmpeg_sidecar::{
    command::FfmpegCommand,
    event::{
        AudioStream, FfmpegDuration, FfmpegEvent, FfmpegProgress, LogLevel, StreamTypeSpecificData,
        VideoStream,
    },
};
use indicatif::ProgressBar;
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
    Other(u32, u32),
}

#[derive(Debug)]
pub enum ToStrError {
    BothAreDynamicValue,
}

impl fmt::Display for ToStrError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ToStrError::BothAreDynamicValue => write!(f, "幅と高さの両方が動的値です."),
        }
    }
}

impl VideoRes {
    pub fn list169() -> Vec<Self> {
        vec![
            VideoRes::R240p,
            VideoRes::R360p,
            VideoRes::R480p,
            VideoRes::R720p,
            VideoRes::R1080p,
            VideoRes::R1440p,
            VideoRes::R2160p,
            VideoRes::R4320p,
        ]
    }

    pub fn to_wh(&self) -> (u32, u32) {
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

    pub fn from_wh(width: u32, height: u32) -> Self {
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

    pub fn from_wh_dynamic(
        width: Option<i32>,
        height: Option<i32>,
        video_stream: VideoStream,
    ) -> Result<Self, ToStrError> {
        let VideoStream {
            width: rw,
            height: rh,
            ..
        } = video_stream;
        let ratio = rw as f32 / rh as f32;
        println!("ratio: {}", ratio);

        if width.is_none() && height.is_none() {
            return Err(ToStrError::BothAreDynamicValue);
        }

        let computed_width = match width {
            Some(w) => w as u32,
            None => {
                let h = height.unwrap();
                (h as f32 * ratio).round() as u32
            }
        };

        let computed_height = match height {
            Some(h) => h as u32,
            None => {
                let w = width.unwrap();
                (w as f32 / ratio).round() as u32
            }
        };

        Ok(VideoRes::from_wh(computed_width, computed_height))
    }

    pub fn to_args(&self) -> String {
        let (width, height) = self.to_wh();
        format!("-s {}x{}", width, height)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_from_wh_dynamic() {
        use super::*;

        let video_stream = VideoStream {
            width: 1920,
            height: 1080,
            fps: 30.0,
            pix_fmt: "yuv420p".to_string(),
        };

        assert_eq!(
            VideoRes::from_wh_dynamic(None, Some(720), video_stream.clone())
                .unwrap()
                .to_wh(),
            VideoRes::R720p.to_wh()
        );
        assert_eq!(
            VideoRes::from_wh_dynamic(Some(1280), None, video_stream.clone())
                .unwrap()
                .to_wh(),
            VideoRes::R720p.to_wh()
        );
        assert_eq!(
            VideoRes::from_wh_dynamic(Some(1280), Some(720), video_stream.clone())
                .unwrap()
                .to_wh(),
            VideoRes::R720p.to_wh()
        );
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
    FfmpegError(String),
    FileError(io::Error),
}

impl fmt::Display for VideoStatErr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            VideoStatErr::NoVideoStreamFound => write!(f, "動画ストリームが見つかりません"),
            VideoStatErr::MultipleVideoStreamFound => {
                write!(f, "動画ストリームが複数見つかりました")
            }
            VideoStatErr::NoDurationFound => write!(f, "動画の長さが取得できません"),
            VideoStatErr::FfmpegError(e) => write!(f, "ffmpegエラー: {}", e),
            VideoStatErr::FileError(e) => write!(f, "ファイルエラー: {}", e),
        }
    }
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

#[derive(Debug)]
pub enum VideoConfigUpScalingErr {
    Resolution(VideoRes, VideoRes),
    Fps(u32, u32),
    HasAudio,
}

impl fmt::Display for VideoConfigUpScalingErr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let msg = match self {
            VideoConfigUpScalingErr::Resolution(c, r) => format!(
                "解像度が元動画より大きいです: {} > {}",
                c.to_name(),
                r.to_name()
            ),
            VideoConfigUpScalingErr::Fps(c, r) => {
                format!("FPSが元動画より大きいです: {} > {}", c, r)
            }
            VideoConfigUpScalingErr::HasAudio => "音声が元動画に含まれていません".to_string(),
        };
        write!(f, "アップスケーリングエラー: {}", msg)
    }
}

#[derive(Debug, Clone)]
pub struct VideoConfig {
    pub res: VideoRes,
    pub fps: u32,
    pub crf: u32,
    pub has_audio: bool,
}

impl VideoConfig {
    pub fn to_file_name(&self) -> String {
        format!(
            "--res-{}--fps-{}--crf-{}",
            self.res.to_file_name(),
            self.fps,
            self.crf
        )
    }

    pub fn check_up_scaling(&self, stat: &VideoStat) -> Result<(), VideoConfigUpScalingErr> {
        let VideoStat {
            video_stream:
                VideoStream {
                    width: r_width,
                    height: r_height,
                    fps: r_fps,
                    ..
                },
            audio_streams,
            ..
        } = stat;

        let VideoConfig {
            res,
            fps: c_fps,
            has_audio,
            ..
        } = self;
        let (c_width, c_height) = res.to_wh();

        if c_width > *r_width || c_height > *r_height {
            return Err(VideoConfigUpScalingErr::Resolution(
                self.res.clone(),
                VideoRes::from_wh(*r_width, *r_height),
            ));
        }

        if *c_fps > *r_fps as u32 {
            return Err(VideoConfigUpScalingErr::Fps(self.fps, *r_fps as u32));
        }

        if *has_audio && audio_streams.is_empty() {
            return Err(VideoConfigUpScalingErr::HasAudio);
        }

        Ok(())
    }
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            res: VideoRes::R720p,
            fps: 30,
            crf: 23,
            has_audio: true,
        }
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
) -> Result<(), String> {
    match level {
        LogLevel::Fatal | LogLevel::Error => {
            let err_body = err.split("[fatal] ").last().unwrap().to_owned();
            let expected = err_body == "At least one output file must be specified";

            if expected && ignore_no_input {
                return Ok(());
            }

            Err(err_body)
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

    if let Err(e) = VideoConfig::check_up_scaling(&config, &stat) {
        return Err(anyhow!(e)).context("エンコード設定に問題があります");
    }

    let arg = config.res.to_args();
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
