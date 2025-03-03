mod modules;

use anyhow::{anyhow, bail, Context, Result};
use console::style;
use humansize::{format_size, DECIMAL};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::iter::zip;

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
    let output_path = format!("out/2--crf-{}.mp4", config.crf);
    let crf = config.crf;

    video::process(
        stat,
        video::VideoProcessParams {
            output_path: output_path.clone(),
            config,
        },
        pb.clone(),
    )
    .await
    .with_context(|| format!("CRF {} でのエンコードに失敗しました.", crf))?;

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
        .expect("元動画の情報の取得に失敗しました");

    let crf_iter = (20..40).step_by(10);

    let progress = MultiProgress::new();
    let spinner_style = get_style(false);

    let tasks = crf_iter.clone().map(|crf| {
        let pb = progress.add(ProgressBar::no_length());
        pb.set_style(spinner_style.clone());
        pb.set_prefix(format!("CRF: {}", crf));

        tokio::spawn({
            let value = stat.clone();
            async move {
                process(
                    value,
                    VideoConfig {
                        res: VideoRes::R1080p.into(),
                        fps: Some(30_f64),
                        crf: 32,
                        has_audio: false,
                    },
                    pb.clone(),
                )
                .await
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
    let results = binding.iter().map(|r| match r {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(anyhow!("{:?}", e)),
        Err(e) => Err(anyhow!("{:?}", e)),
    });

    println!();
    println!();
    results.clone().all(|r| r.is_ok()).then(|| {
        println!("{}", style("✓ すべて正常にエンコードしました！").green());
    });

    for (crf, result) in zip(crf_iter.clone(), results.clone()) {
        match result {
            Ok(()) => {}
            Err(e) => {
                bail!("[CRF: {}] エンコード失敗: {:?}", crf, e);
            }
        }
    }

    Ok(())
}
