// xore.project manifest — project model and parser.
//
// Every Xore project has a single `xore.project` file at the root.
// Format is TOML-like but we parse it manually to keep zero heavy deps.
//
// Example xore.project:
//
//   [project]
//   name    = "hello"
//   version = "0.1.0"
//   entry   = "src/main.xre"
//   type    = "bin"           # bin | lib | osdev
//
//   [build]
//   target   = "x86_64-linux" # x86_64-linux | x86_64-windows | bare-metal
//   optimize = "debug"        # debug | release
//
//   [libs]
//   std = "libs/std"
//   mylib = "libs/mylib"

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{fmt, fs};

// ─── Project types ───────────────────────────────────────────────────────────

/// What the project compiles to.
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectType {
    /// Standard executable — linked against libc, produces ELF/PE.
    Bin,
    /// Compiled library — produces .libx archive.
    Lib,
    /// Bare-metal / OS-dev — no libc, produces raw .elf.
    Osdev,
}

impl ProjectType {
    fn from_str(s: &str) -> Result<Self, ProjectError> {
        match s {
            "bin"   => Ok(Self::Bin),
            "lib"   => Ok(Self::Lib),
            "osdev" => Ok(Self::Osdev),
            other   => Err(ProjectError::InvalidField {
                field: "type".into(),
                value: other.into(),
                hint:  "expected: bin | lib | osdev".into(),
            }),
        }
    }
}

/// Compilation target triple.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    X86_64Linux,
    X86_64Windows,
    BareMetal,
}

impl Target {
    fn from_str(s: &str) -> Result<Self, ProjectError> {
        match s {
            "x86_64-linux"   => Ok(Self::X86_64Linux),
            "x86_64-windows" => Ok(Self::X86_64Windows),
            "bare-metal"     => Ok(Self::BareMetal),
            other => Err(ProjectError::InvalidField {
                field: "target".into(),
                value: other.into(),
                hint:  "expected: x86_64-linux | x86_64-windows | bare-metal".into(),
            }),
        }
    }

    /// LLVM target triple string.
    pub fn llvm_triple(&self) -> &'static str {
        match self {
            Self::X86_64Linux   => "x86_64-unknown-linux-gnu",
            Self::X86_64Windows => "x86_64-pc-windows-msvc",
            Self::BareMetal     => "x86_64-unknown-none",
        }
    }
}

/// Optimisation level.
#[derive(Debug, Clone, PartialEq)]
pub enum OptLevel {
    Debug,    // -O0 — fast build, full debug info
    Release,  // -O3 — full optimisations
}

impl OptLevel {
    fn from_str(s: &str) -> Result<Self, ProjectError> {
        match s {
            "debug"   => Ok(Self::Debug),
            "release" => Ok(Self::Release),
            other => Err(ProjectError::InvalidField {
                field: "optimize".into(),
                value: other.into(),
                hint:  "expected: debug | release".into(),
            }),
        }
    }

    pub fn llvm_opt_flag(&self) -> &'static str {
        match self { Self::Debug => "-O0", Self::Release => "-O3" }
    }
}

// ─── Manifest ────────────────────────────────────────────────────────────────

/// Fully parsed `xore.project` manifest.
#[derive(Debug, Clone)]
pub struct Manifest {
    // [project]
    pub name:    String,
    pub version: String,
    pub entry:   PathBuf,
    pub ty:      ProjectType,

    // [build]
    pub target:   Target,
    pub optimize: OptLevel,

    // [libs]  name → path
    pub libs: HashMap<String, PathBuf>,

    /// Root directory of the project (directory containing xore.project).
    pub root: PathBuf,
}

impl Manifest {
    /// Load and parse `xore.project` from `project_root`.
    pub fn load(project_root: &Path) -> Result<Self, ProjectError> {
        let manifest_path = project_root.join("xore.project");
        let src = fs::read_to_string(&manifest_path)
            .map_err(|_| ProjectError::ManifestNotFound(manifest_path.clone()))?;
        Self::parse(&src, project_root)
    }

    /// Parse manifest source text.
    pub fn parse(src: &str, root: &Path) -> Result<Self, ProjectError> {
        let sections = parse_sections(src)?;

        // ── [project] ──────────────────────────────────────────────────────
        let proj = sections.get("project")
            .ok_or_else(|| ProjectError::MissingSection("project".into()))?;

        let name = proj.get("name")
            .ok_or_else(|| ProjectError::MissingField("project.name".into()))?.clone();

        let version = proj.get("version")
            .cloned().unwrap_or_else(|| "0.1.0".into());

        let entry_str = proj.get("entry")
            .cloned().unwrap_or_else(|| "src/main.xre".into());
        let entry = root.join(&entry_str);

        let ty = proj.get("type")
            .map(|s| ProjectType::from_str(s))
            .unwrap_or(Ok(ProjectType::Bin))?;

        // ── [build] ────────────────────────────────────────────────────────
        let build = sections.get("build");

        let target = build.and_then(|b| b.get("target"))
            .map(|s| Target::from_str(s))
            .unwrap_or(Ok(Target::X86_64Linux))?;

        let optimize = build.and_then(|b| b.get("optimize"))
            .map(|s| OptLevel::from_str(s))
            .unwrap_or(Ok(OptLevel::Debug))?;

        // ── [libs] ─────────────────────────────────────────────────────────
        let mut libs = HashMap::new();
        if let Some(lib_section) = sections.get("libs") {
            for (k, v) in lib_section {
                libs.insert(k.clone(), root.join(v));
            }
        }

        Ok(Manifest { name, version, entry, ty, target, optimize, libs, root: root.to_path_buf() })
    }

    /// Directory for build artifacts.
    pub fn build_dir(&self) -> PathBuf {
        self.root.join("build")
    }

    /// Path of the output binary / library.
    pub fn output_path(&self) -> PathBuf {
        match &self.ty {
            ProjectType::Bin   => self.build_dir().join(&self.name),
            ProjectType::Lib   => self.build_dir().join(format!("{}.libx", self.name)),
            ProjectType::Osdev => self.build_dir().join(format!("{}.elf", self.name)),
        }
    }
}

// ─── Module resolution ────────────────────────────────────────────────────────

/// A source unit — either a standalone .xre or a paired .xrs/.xrb module.
#[derive(Debug, Clone)]
pub enum SourceUnit {
    /// Single `.xre` file (regular code).
    Single(PathBuf),
    /// Critical module: must have both spec (.xrs) and body (.xrb).
    Module { spec: PathBuf, body: PathBuf, name: String },
}

impl SourceUnit {
    pub fn name(&self) -> &str {
        match self {
            Self::Single(p) => p.file_stem()
                .and_then(|s| s.to_str()).unwrap_or("unknown"),
            Self::Module { name, .. } => name,
        }
    }
}

/// Walk `src/` and collect all source units.
/// - Pairs `.xrs` + `.xrb` with the same stem → `SourceUnit::Module`
/// - Lone `.xre` files → `SourceUnit::Single`
/// - A `.xrs` without a `.xrb` (or vice versa) → error
pub fn collect_sources(src_dir: &Path) -> Result<Vec<SourceUnit>, ProjectError> {
    let mut specs: HashMap<String, PathBuf> = HashMap::new();
    let mut bodies: HashMap<String, PathBuf> = HashMap::new();
    let mut singles: Vec<PathBuf> = Vec::new();

    let entries = fs::read_dir(src_dir)
        .map_err(|_| ProjectError::SrcDirNotFound(src_dir.to_path_buf()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        let ext  = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let stem = path.file_stem().and_then(|s| s.to_str())
            .unwrap_or("").to_string();

        match ext {
            "xre" => singles.push(path),
            "xrs" => { specs.insert(stem, path); }
            "xrb" => { bodies.insert(stem, path); }
            _     => {}
        }
    }

    let mut units: Vec<SourceUnit> = Vec::new();

    // Pair up .xrs and .xrb
    for (stem, spec_path) in &specs {
        match bodies.get(stem) {
            Some(body_path) => units.push(SourceUnit::Module {
                spec: spec_path.clone(),
                body: body_path.clone(),
                name: stem.clone(),
            }),
            None => return Err(ProjectError::UnpairedSpec {
                spec: spec_path.clone(),
                missing: src_dir.join(format!("{stem}.xrb")),
            }),
        }
    }

    // Check for orphaned .xrb files
    for (stem, body_path) in &bodies {
        if !specs.contains_key(stem) {
            return Err(ProjectError::UnpairedBody {
                body: body_path.clone(),
                missing: src_dir.join(format!("{stem}.xrs")),
            });
        }
    }

    // Add standalone .xre files
    for p in singles { units.push(SourceUnit::Single(p)); }

    Ok(units)
}

// ─── Minimal TOML-like section parser ────────────────────────────────────────
// Full TOML is overkill — we only need [section] + key = "value" pairs.

fn parse_sections(src: &str) -> Result<HashMap<String, HashMap<String, String>>, ProjectError> {
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current = String::new();

    for (lineno, raw_line) in src.lines().enumerate() {
        let line = raw_line.trim();

        // Skip blank lines and comments
        if line.is_empty() || line.starts_with('#') { continue; }

        // Section header: [name]
        if line.starts_with('[') && line.ends_with(']') {
            current = line[1..line.len()-1].trim().to_string();
            sections.entry(current.clone()).or_default();
            continue;
        }

        // Key = "value" pair
        if let Some(eq) = line.find('=') {
            let key   = line[..eq].trim().to_string();
            let raw_v = line[eq+1..].trim();
            // Strip surrounding quotes
            let value = if (raw_v.starts_with('"') && raw_v.ends_with('"'))
                || (raw_v.starts_with('\'') && raw_v.ends_with('\''))
            {
                raw_v[1..raw_v.len()-1].to_string()
            } else {
                // Unquoted value (e.g.  optimize = debug)
                // Strip inline comment
                raw_v.splitn(2, '#').next().unwrap_or("").trim().to_string()
            };

            if current.is_empty() {
                return Err(ProjectError::ParseError {
                    line: lineno + 1,
                    msg:  "key-value pair outside any [section]".into(),
                });
            }
            sections.entry(current.clone()).or_default().insert(key, value);
        } else {
            return Err(ProjectError::ParseError {
                line: lineno + 1,
                msg:  format!("cannot parse line: `{line}`"),
            });
        }
    }

    Ok(sections)
}

// ─── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ProjectError {
    ManifestNotFound(PathBuf),
    SrcDirNotFound(PathBuf),
    MissingSection(String),
    MissingField(String),
    InvalidField { field: String, value: String, hint: String },
    ParseError { line: usize, msg: String },
    UnpairedSpec { spec: PathBuf, missing: PathBuf },
    UnpairedBody { body: PathBuf, missing: PathBuf },
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ManifestNotFound(p) =>
                write!(f, "cannot find `xore.project` in `{}`", p.display()),
            Self::SrcDirNotFound(p) =>
                write!(f, "source directory not found: `{}`", p.display()),
            Self::MissingSection(s) =>
                write!(f, "xore.project: missing required section `[{s}]`"),
            Self::MissingField(k) =>
                write!(f, "xore.project: missing required field `{k}`"),
            Self::InvalidField { field, value, hint } =>
                write!(f, "xore.project: invalid value `{value}` for `{field}` — {hint}"),
            Self::ParseError { line, msg } =>
                write!(f, "xore.project:{line}: {msg}"),
            Self::UnpairedSpec { spec, missing } =>
                write!(f, "spec file `{}` has no matching body `{}` — critical modules require both .xrs and .xrb",
                    spec.display(), missing.display()),
            Self::UnpairedBody { body, missing } =>
                write!(f, "body file `{}` has no matching spec `{}` — critical modules require both .xrs and .xrb",
                    body.display(), missing.display()),
        }
    }
}

impl std::error::Error for ProjectError {}

// ─── Project scaffolding (xore new) ──────────────────────────────────────────

/// Create a new Xore project directory tree.
pub fn scaffold_project(name: &str, root: &Path, ty: ProjectType) -> Result<(), ProjectError> {
    let src = root.join("src");
    let libs = root.join("libs");
    let build = root.join("build");

    fs::create_dir_all(&src).ok();
    fs::create_dir_all(&libs).ok();
    fs::create_dir_all(&build).ok();

    // xore.project
    let type_str = match ty { ProjectType::Bin => "bin", ProjectType::Lib => "lib", ProjectType::Osdev => "osdev" };
    let manifest = format!(
r#"[project]
name    = "{name}"
version = "0.1.0"
entry   = "src/main.xre"
type    = "{type_str}"

[build]
target   = "x86_64-linux"
optimize = "debug"

[libs]
# std = "libs/std"
"#);
    fs::write(root.join("xore.project"), manifest).ok();

    // src/main.xre
    let main_xr = format!(
r#"// {name} — main.xre

public main() {{
    println!("Hello from {name}!");
}}
"#);
    fs::write(src.join("main.xre"), main_xr).ok();

    // .gitignore
    fs::write(root.join(".gitignore"), "build/\n*.ll\n*.o\n").ok();

    Ok(())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const SAMPLE: &str = r#"
[project]
name    = "hello"
version = "0.1.0"
entry   = "src/main.xre"
type    = "bin"

[build]
target   = "x86_64-linux"
optimize = "debug"

[libs]
std = "libs/std"
"#;

    #[test]
    fn parse_full_manifest() {
        let m = Manifest::parse(SAMPLE, Path::new("/tmp")).unwrap();
        assert_eq!(m.name, "hello");
        assert_eq!(m.version, "0.1.0");
        assert_eq!(m.ty, ProjectType::Bin);
        assert_eq!(m.target, Target::X86_64Linux);
        assert_eq!(m.optimize, OptLevel::Debug);
        assert!(m.libs.contains_key("std"));
    }

    #[test]
    fn missing_project_section() {
        let src = "[build]\ntarget = \"x86_64-linux\"\n";
        assert!(Manifest::parse(src, Path::new("/tmp")).is_err());
    }

    #[test]
    fn missing_name_field() {
        let src = "[project]\nversion = \"1.0\"\n";
        assert!(Manifest::parse(src, Path::new("/tmp")).is_err());
    }

    #[test]
    fn invalid_project_type() {
        let src = "[project]\nname = \"x\"\ntype = \"unknown\"\n";
        assert!(Manifest::parse(src, Path::new("/tmp")).is_err());
    }

    #[test]
    fn defaults_when_optional_missing() {
        let src = "[project]\nname = \"minimal\"\n";
        let m = Manifest::parse(src, Path::new("/tmp")).unwrap();
        assert_eq!(m.ty, ProjectType::Bin);
        assert_eq!(m.target, Target::X86_64Linux);
        assert_eq!(m.optimize, OptLevel::Debug);
    }

    #[test]
    fn osdev_type_parses() {
        let src = "[project]\nname = \"kernel\"\ntype = \"osdev\"\n";
        let m = Manifest::parse(src, Path::new("/tmp")).unwrap();
        assert_eq!(m.ty, ProjectType::Osdev);
    }

    #[test]
    fn output_path_bin() {
        let src = "[project]\nname = \"app\"\ntype = \"bin\"\n";
        let m = Manifest::parse(src, Path::new("/proj")).unwrap();
        assert!(m.output_path().to_str().unwrap().ends_with("app"));
    }

    #[test]
    fn output_path_lib() {
        let src = "[project]\nname = \"mylib\"\ntype = \"lib\"\n";
        let m = Manifest::parse(src, Path::new("/proj")).unwrap();
        assert!(m.output_path().to_str().unwrap().ends_with("mylib.libx"));
    }

    #[test]
    fn output_path_osdev() {
        let src = "[project]\nname = \"kern\"\ntype = \"osdev\"\n";
        let m = Manifest::parse(src, Path::new("/proj")).unwrap();
        assert!(m.output_path().to_str().unwrap().ends_with("kern.elf"));
    }

    #[test]
    fn llvm_triple_correct() {
        assert_eq!(Target::X86_64Linux.llvm_triple(),   "x86_64-unknown-linux-gnu");
        assert_eq!(Target::X86_64Windows.llvm_triple(), "x86_64-pc-windows-msvc");
        assert_eq!(Target::BareMetal.llvm_triple(),     "x86_64-unknown-none");
    }

    #[test]
    fn comments_and_blank_lines_ignored() {
        let src = "\n# this is a comment\n[project]\n# another comment\nname = \"test\"\n";
        let m = Manifest::parse(src, Path::new("/tmp")).unwrap();
        assert_eq!(m.name, "test");
    }

    #[test]
    fn unquoted_value_parses() {
        let src = "[project]\nname = hello\n";
        let m = Manifest::parse(src, Path::new("/tmp")).unwrap();
        assert_eq!(m.name, "hello");
    }
}