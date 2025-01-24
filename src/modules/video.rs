use anyhow::{bail, Result};
use ffmpeg_sidecar::{
    command::FfmpegCommand,
    event::{FfmpegDuration, FfmpegEvent, FfmpegProgress, LogLevel, StreamTypeSpecificData},
};
use humansize::ToF64;
use indicatif::ProgressBar;

pub async fn process(input_path: &str, output_path: &str, crf: u32, pb: ProgressBar) -> Result<()> {
    let mut runner = FfmpegCommand::new()
        .input(input_path)
        .crf(crf)
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
