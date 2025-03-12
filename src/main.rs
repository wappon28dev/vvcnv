mod modules;

use anyhow::{anyhow, bail, Context, Result};
use console::style;
use ffmpeg_sidecar::event::VideoStream;
use humansize::{format_size, DECIMAL};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use itertools::iproduct;
use std::{
    iter::{self, zip},
    ops,
};

use modules::{
    file,
    video::{self, VideoConfig, VideoRes, VideoStat},
};

fn get_style(is_done: bool) -> ProgressStyle {
    ProgressStyle::with_template(&format!(
        "\n{} -> {}\n  {}\n  {}{} {} {} | {}{} | {}",
        style("{spinner}").blue(),
        style("{prefix}"),
        "{bar:40.cyan/blue}",
        "{pos:>3}",
        style("/{len:>3}").dim(),
        style("[fr]").dim(),
        style(format!("({})", style("{percent:>3}%").for_stdout())).dim(),
        "{elapsed_precise}",
        if is_done {
            style("/{elapsed_precise}").dim()
        } else {
            style("/{duration_precise}").dim()
        },
        style("{msg}")
    ))
    .unwrap()
    .progress_chars("=>-")
}

async fn process(stat: VideoStat, config: VideoConfig, pb: ProgressBar) -> Result<()> {
    let (name, ext) = file::get_file_name(&stat.path);
    let output_path = format!("out/{}{}.{}", name, config.to_file_name(), ext);

    video::process(
        stat,
        video::VideoProcessParams {
            output_path: output_path.clone(),
            config: config.clone(),
        },
        pb.clone(),
    )
    .await?;

    let output_size =
        file::calc_size(&output_path).context("出力動画のサイズの取得に失敗しました.")?;
    let output_size_str = format_size(output_size, DECIMAL);

    pb.set_style(get_style(true));
    pb.finish_with_message(format!(
        "{}: {}",
        style("✓ エンコード完了").green(),
        style(output_size_str).green().bright()
    ));

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let input_path = "assets/2.mp4";

    let stat = video::stat(input_path.to_string())
        .await
        .map_err(|e| anyhow!(e).context("元動画の情報取得に失敗しました."))?;

    let res_iter = VideoRes::list169();
    let fps_iter = (30..=30).step_by(30).collect::<Vec<_>>();
    let crf_iter = (20..=40).step_by(20).collect::<Vec<_>>();
    // let res_iter = (480..=1080)
    //     .step_by(240)
    //     .map(|h| VideoRes::from_wh_dynamic(None, Some(h), stat.video_stream.clone()))
    //     .map(Result::unwrap)
    //     .collect::<Vec<_>>();

    let iter_prod = iproduct!(res_iter, fps_iter, crf_iter);

    let progress = MultiProgress::new();
    let spinner_style = get_style(false);

    let tasks = iter_prod.clone().map(|(res, fps, crf)| {
        let pb = progress.add(ProgressBar::no_length());
        pb.set_style(spinner_style.clone());
        pb.set_prefix(format!("RES: {:?}, FPS: {}, CRF: {}", res, fps, crf));

        tokio::spawn({
            let value = stat.clone();
            let config = VideoConfig {
                crf,
                fps,
                res,
                has_audio: true,
            };

            async move {
                process(value, config, pb.clone()).await.inspect_err(|e| {
                    pb.finish_with_message(format!(
                        "{}: {}",
                        style("✗ エンコード失敗").red(),
                        style(&e).red().bright()
                    ));
                })
            }
        })
    });

    println!(
        "{}",
        style(format!(
            "元動画のサイズ: {}",
            format_size(stat.file_size, DECIMAL)
        ))
        .dim()
    );

    let binding = futures::future::join_all(tasks).await;
    let results = binding
        .iter()
        .map(|r| r.as_ref().unwrap())
        .collect::<Vec<_>>();

    println!();
    println!();
    results.clone().iter().all(|r| r.is_ok()).then(|| {
        println!("{}", style("✓ すべて正常にエンコードしました！").green());
    });
    zip(iter_prod.clone(), results.clone())
        .clone()
        .filter(|(_, r)| r.is_err())
        .for_each(|((res, fps, crf), e)| {
            eprintln!(
                "\n{}\n{}:\n{:?}",
                style("--------------------").dim(),
                style(format!(
                    "✗ エンコード失敗 - RES: {:?}, FPS: {}, CRF: {}",
                    res, fps, crf
                ))
                .red(),
                style(e.as_ref().unwrap_err()).red().bright()
            );
        });

    Ok(())
}
