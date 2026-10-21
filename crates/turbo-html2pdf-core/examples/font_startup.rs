//! Process-cold eager system-font timing and selection/shaping fingerprints.
//! Run in a fresh process; pass the installed font directories to fingerprint:
//! `cargo run --release -p turbo-html2pdf-core --example font_startup -- /System/Library/Fonts /System/Library/Fonts/Supplemental /Library/Fonts "$HOME/Library/Fonts"`
//! The registry always uses its normal host discovery policy. Directory arguments
//! select diagnostic inputs only. No arguments measures generic-family controls.
//! `/usr/bin/time -l` on macOS reports peak RSS, including later fingerprinting;
//! the JSON initialization times exclude fingerprinting. OS disk caches may be warm.
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use turbo_html2pdf_core::{FontFace, FontRegistry};

type Query = (String, u16, bool);

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn family(face: &rustybuzz::ttf_parser::Face<'_>) -> Option<String> {
    let unicode = |id| {
        face.names()
            .into_iter()
            .filter(|name| name.is_unicode() && name.name_id == id)
            .find_map(|name| name.to_string())
    };
    unicode(16)
        .or_else(|| unicode(1))
        .or_else(|| platform_family(face))
}

fn platform_family(face: &rustybuzz::ttf_parser::Face<'_>) -> Option<String> {
    let name = face.names().into_iter().find(|name| {
        name.name_id == 1
            && name.platform_id == rustybuzz::ttf_parser::name::PlatformId::Macintosh
            && name.encoding_id == 0
    })?;
    Some(encoding_rs::MACINTOSH.decode(name.name).0.into_owned())
}

fn is_font(path: &Path) -> bool {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "ttc" | "otf")
}

fn font_paths(dirs: &[PathBuf]) -> Vec<PathBuf> {
    dirs.iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_font(path))
        .collect()
}

fn describe_query(bytes: &[u8], index: u32) -> Option<Query> {
    let face = rustybuzz::ttf_parser::Face::parse(bytes, index).ok()?;
    Some((family(&face)?, face.weight().to_number(), face.is_italic()))
}

fn describe_file(path: &Path) -> Option<(Value, Vec<Query>)> {
    let bytes = std::fs::read(path).ok()?;
    let count = rustybuzz::ttf_parser::fonts_in_collection(&bytes).unwrap_or(1);
    let queries = (0..count)
        .filter_map(|index| describe_query(&bytes, index))
        .collect();
    Some((
        json!({"path":path,"bytes":bytes.len(),"sha256":hash(&bytes),"faceCount":count}),
        queries,
    ))
}

fn diagnostic_inputs(dirs: &[PathBuf]) -> (Vec<Value>, BTreeSet<Query>) {
    let mut files = Vec::new();
    let mut queries = BTreeSet::new();
    for path in font_paths(dirs) {
        if let Some((file, names)) = describe_file(&path) {
            files.push(file);
            queries.extend(names);
        }
    }
    files.sort_by_key(|file| file["path"].as_str().unwrap().to_owned());
    for name in ["serif", "sans-serif", "monospace"] {
        for (weight, italic) in [(400, false), (700, false), (400, true), (700, true)] {
            queries.insert((name.into(), weight, italic));
        }
    }
    (files, queries)
}

#[derive(Default)]
struct Fingerprints {
    programs: HashMap<usize, String>,
    backing_bytes: u64,
}

impl Fingerprints {
    fn program(&mut self, face: &FontFace) -> String {
        self.programs
            .entry(face.data().as_ptr() as usize)
            .or_insert_with(|| {
                self.backing_bytes += face.data().len() as u64;
                hash(face.data())
            })
            .clone()
    }

    fn selected(&mut self, registry: &FontRegistry, query: &Query) -> Value {
        let (name, weight, italic) = query;
        let Some(face) = registry.select(&[name], *weight, *italic) else {
            return json!({"requested":query,"selected":null});
        };
        let metrics = face.html_metrics(18.72);
        let text = "AV office ffi العربية א🙂";
        let glyphs: Vec<_> = face
            .shape(text)
            .into_iter()
            .map(|g| {
                [
                    i64::from(g.glyph_id),
                    i64::from(g.x_advance),
                    i64::from(g.x_offset),
                    i64::from(g.y_offset),
                    i64::from(g.cluster),
                ]
            })
            .collect();
        json!({"requested":query,"selected":[face.family(),&face.weight().to_string(),&face.is_italic().to_string()],"index":face.index(),"programSha256":self.program(face),"unitsPerEm":face.units_per_em(),"bbox":face.bbox(),"htmlMetrics":[metrics.ascent,metrics.descent,metrics.line_gap],"measure":face.measure(text,18.72,0.0),"glyphs":glyphs})
    }
}

fn main() {
    let started = Instant::now();
    let mut registry = FontRegistry::new();
    let bundled_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    registry.load_system_fonts_for_html();
    let system_ms = started.elapsed().as_secs_f64() * 1000.0;
    let dirs: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let (files, queries) = diagnostic_inputs(&dirs);
    let mut fingerprints = Fingerprints::default();
    let rows: Vec<_> = queries
        .iter()
        .map(|query| fingerprints.selected(&registry, query))
        .collect();
    println!(
        "{}",
        json!({"bundledInitMs":bundled_ms,"systemInitMs":system_ms,"loadedCallerFaceCount":registry.len(),"selectedBackingAllocations":fingerprints.programs.len(),"selectedBackingBytes":fingerprints.backing_bytes,"selectedQueryCount":rows.len(),"selectionAndShapingSha256":hash(&serde_json::to_vec(&rows).unwrap()),"fontFileManifestSha256":hash(&serde_json::to_vec(&files).unwrap()),"fontDirectoryInputs":dirs,"fontFiles":files,"rows":rows})
    );
}
