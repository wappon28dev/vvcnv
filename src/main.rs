mod modules;

use anyhow::{anyhow, bail, Context, Result};
use console::style;
use humansize::{format_size, DECIMAL};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::iter::zip;

use modules::{file, video};

async fn process(input_path: &str, crf: u32, pb: ProgressBar) -> Result<()> {
    let output_path = format!("out/2--crf-{}.mp4", crf);

    video::process(input_path, &output_path, crf, pb.clone())
        .await
        .with_context(|| format!("CRF {} でのエンコードに失敗しました.", crf))?;

    let output_size =
        file::calc_size(&output_path).context("出力動画のサイズの取得に失敗しました.")?;
    let output_size_str = format_size(output_size, DECIMAL);

    pb.finish_with_message(format!("✓ エンコード完了: {}", output_size_str));

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let input_path = "assets/2.mp4";

    let crf_iter = (20..40).step_by(10);

    let progress = MultiProgress::new();
    let spinner_style = ProgressStyle::with_template(
        "{spinner:.green.dim} {bar:40.cyan/blue} {pos:>3}/{len:>3.gray} [fr] ({percent}%) - {elapsed_precise}/{duration_precise} - {msg}"
    )
    .unwrap()
    .progress_chars("█▓▒░");

    let tasks = crf_iter.clone().map(|crf| {
        let pb = progress.add(ProgressBar::no_length());
        pb.set_style(spinner_style.clone());
        pb.set_prefix(format!("CRF: {}", crf));

        tokio::spawn(async move { process(input_path, crf, pb.clone()).await })
    });

    let origin_size = file::calc_size(input_path).context("元動画のサイズの取得に失敗しました.")?;
    println!("元動画のサイズ: {}", format_size(origin_size, DECIMAL));

    let binding = futures::future::join_all(tasks).await;
    let results = binding.iter().map(|r| match r {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(anyhow!("{:?}", e)),
        Err(e) => Err(anyhow!("{:?}", e)),
    });

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
