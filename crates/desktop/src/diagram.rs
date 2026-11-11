//! A diagram you asked for, drawn: its Mermaid laid out and drawn as an SVG
//! (mermaid-rs-renderer, in this process: no browser), then a PNG at twice
//! the size for pasting. The last few are kept beside the models, where a
//! pasted file stays readable after the paste.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use localflow_core::diagram::{Diagram, Kind};
use localflow_core::local::models_dir;

/// How many drawn diagrams are kept.
const KEEP: usize = 50;

/// ~/.local/share/localflow/diagrams, ~/Library/Application
/// Support/localflow/diagrams, %LOCALAPPDATA%\localflow\diagrams.
pub fn diagrams_dir() -> PathBuf {
    let models = models_dir();
    models.parent().unwrap_or(&models).join("diagrams")
}

/// The diagram as an SVG (an error when the Mermaid doesn't parse).
pub fn svg(mermaid: &str) -> Result<String, String> {
    mermaid_rs_renderer::render_strict(mermaid, Default::default()).map_err(|e| e.to_string())
}

/// The SVG as a PNG, `scale` times its size, on white.
pub fn png(svg: &str, scale: f32) -> Result<Vec<u8>, String> {
    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_str(svg, &options).map_err(|e| e.to_string())?;
    let size = tree.size().to_int_size().scale_by(scale).ok_or("the diagram is empty")?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height()).ok_or("the diagram is too big")?;
    pixmap.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|e| e.to_string())
}

/// For a code editor, a terminal or a chat that renders Markdown.
pub fn fenced(mermaid: &str) -> String {
    format!("```mermaid\n{}\n```", mermaid.trim_end())
}

/// A diagram drawn and saved: the PNG to paste, and its Mermaid beside it.
#[derive(Debug, Clone)]
pub struct Drawn {
    pub png: PathBuf,
    pub mermaid: PathBuf,
}

/// Draw `diagram` and save it in `dir` ("flowchart-1762270000.png" and
/// ".mmd"), keeping the newest KEEP.
pub fn save_in(dir: &Path, diagram: &Diagram) -> Result<Drawn, String> {
    let png = png(&svg(&diagram.mermaid)?, 2.0)?;
    let io = |e: std::io::Error| format!("{}: {e}", dir.display());
    std::fs::create_dir_all(dir).map_err(io)?;
    let kind = match diagram.kind {
        Kind::Flowchart => "flowchart",
        Kind::Mindmap => "mindmap",
        Kind::Sequence => "sequence",
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let stem = format!("{kind}-{}{:03}", now.as_secs(), now.subsec_millis());
    let drawn = Drawn { png: dir.join(format!("{stem}.png")), mermaid: dir.join(format!("{stem}.mmd")) };
    std::fs::write(&drawn.png, png).map_err(io)?;
    std::fs::write(&drawn.mermaid, &diagram.mermaid).map_err(io)?;
    prune(dir);
    Ok(drawn)
}

pub fn save(diagram: &Diagram) -> Result<Drawn, String> {
    save_in(&diagrams_dir(), diagram)
}

/// Remove all but the newest KEEP diagrams (with their Mermaid).
fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut pngs: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .map(|p| (std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH), p))
        .collect();
    pngs.sort();
    for (_, old) in pngs.iter().rev().skip(KEEP) {
        let _ = std::fs::remove_file(old);
        let _ = std::fs::remove_file(old.with_extension("mmd"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_each_kind() {
        let dir = std::env::temp_dir().join(format!("localflow-diagrams-{}", std::process::id()));
        for (kind, mermaid) in [
            (Kind::Flowchart, "flowchart TD\n    N1([\"Sign in\"])\n    N2{\"Password right?\"}\n    N3[\"Dashboard\"]\n    \
                               N1 --> N2\n    N2 -->|Yes| N3\n"),
            (Kind::Mindmap, "mindmap\n  root((Trip))\n    Tokyo\n      Shibuya\n    Kyoto\n"),
            (Kind::Sequence, "sequenceDiagram\n    App->>API: Get user\n    API-->>App: User\n"),
        ] {
            let drawn = save_in(&dir, &Diagram::new(kind, mermaid.into())).unwrap();
            let png = std::fs::read(&drawn.png).unwrap();
            assert_eq!(&png[1..4], b"PNG");
            let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
            assert!(width > 100, "{kind:?}: {width} px wide");
            assert_eq!(std::fs::read_to_string(&drawn.mermaid).unwrap(), mermaid);
        }
        assert!(svg("not mermaid at all").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keeps_the_newest() {
        let dir = std::env::temp_dir().join(format!("localflow-prune-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..KEEP + 3 {
            let png = dir.join(format!("flowchart-{i:03}.png"));
            std::fs::write(&png, b"x").unwrap();
            std::fs::write(png.with_extension("mmd"), b"x").unwrap();
            let t = UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 + i as u64);
            std::fs::File::options().write(true).open(&png).unwrap().set_modified(t).unwrap();
        }
        prune(&dir);
        let left = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(left, KEEP * 2);
        assert!(!dir.join("flowchart-000.png").exists() && !dir.join("flowchart-000.mmd").exists());
        assert!(dir.join(format!("flowchart-{:03}.png", KEEP + 2)).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fenced_for_editors() {
        assert_eq!(fenced("flowchart TD\n    A --> B\n"), "```mermaid\nflowchart TD\n    A --> B\n```");
    }
}
