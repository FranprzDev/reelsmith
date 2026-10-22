//! A procedural sample library (no recorded footage): four 6 s clips drawn by FFmpeg's own sources and two alpha motion
//! graphics with a short sound each, so the whole flow can be tried, demoed and smoke-tested without client media.
use crate::clips::{self, ClipList};
use anyhow::{Context, Result};
use std::path::Path;

/// (file name, ffmpeg arguments before the output path)
fn recipes() -> Vec<(&'static str, Vec<&'static str>)> {
    let clip = ["-t", "6", "-r", "30", "-c:v", "libx264", "-crf", "20", "-preset", "veryfast", "-pix_fmt", "yuv420p"];
    let motion_audio = ["-f", "lavfi", "-i", "sine=frequency=880:duration=0.35,afade=t=out:st=0.1:d=0.25"];
    let motion = ["-map", "0:v", "-map", "1:a", "-c:v", "qtrle", "-c:a", "pcm_s16le"];
    let with = |src: &'static str, tail: &[&'static str]| -> Vec<&'static str> { [vec!["-f", "lavfi", "-i", src], tail.to_vec()].concat() };
    vec![
        ("nebula.mp4", with("mandelbrot=s=1280x720:rate=30:end_scale=0.05:end_pts=180:inner=mincol", &clip)),
        ("cells.mp4", with("life=s=320x180:rate=30:mold=10:ratio=0.35:death_color=#101a33:life_color=#3ee8b5,scale=1280:720:flags=neighbor", &clip)),
        ("aurora.mp4", with("gradients=s=1280x720:rate=30:c0=0x2b1055:c1=0x7597de:c2=0xf72585:nb_colors=3:speed=0.03", &clip)),
        ("grid.mp4", with("testsrc2=s=1280x720:rate=30", &clip)),
        (
            "badge.mov",
            [
                vec!["-f", "lavfi", "-i", "color=c=black@0.0:s=640x220:r=30:d=2,format=rgba,drawbox=x=10:y=10:w=620:h=200:color=0xffd166@1:t=fill,drawbox=x=40:y=40:w=560:h=140:color=0x1b1b3a@1:t=fill,fade=t=in:st=0:d=0.3:alpha=1"],
                motion_audio.to_vec(),
                motion.to_vec(),
            ]
            .concat(),
        ),
        (
            "cta.mov",
            [
                vec!["-f", "lavfi", "-i", "color=c=black@0.0:s=1080x1920:r=30:d=4,format=rgba,drawbox=x=140:y=1420:w=800:h=180:color=0xffd166@1:t=fill,drawbox=x=170:y=1450:w=740:h=120:color=0x1b1b3a@1:t=fill,fade=t=in:st=0:d=0.4:alpha=1"],
                motion_audio.to_vec(),
                motion.to_vec(),
            ]
            .concat(),
        ),
    ]
}

/// Write the sample library into `folder` (created when missing) and list it.
pub fn make(folder: &Path) -> Result<ClipList> {
    clips::ensure_ffmpeg()?;
    std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
    for (name, args) in recipes() {
        let out = folder.join(name);
        let mut a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        a.push(out.display().to_string());
        datavid::media::ffmpeg(&a, &format!("the sample {name}"))?;
    }
    clips::list(folder)
}
