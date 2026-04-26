// .libx — Xore compiled library format
//
// Binary layout:
//
//   [0..4]   magic      "LIBX"
//   [4..8]   version    u32 le  (currently 1)
//   [8..12]  name_len   u32 le
//   [12..]   name       UTF-8 string (library name)
//   ...      sections   (repeated)
//
// Each section:
//   [0..4]   tag        u32 le  — section type (see SectionTag)
//   [4..8]   data_len   u32 le
//   [8..]    data       raw bytes
//
// Section types:
//   0x01  OBJECT    — raw ELF object (.o) bytes
//   0x02  SIGS      — exported function signatures (UTF-8 text, one per line)
//   0x03  STRUCTS   — struct layout metadata (UTF-8 text)
//   0x04  ENUMS     — enum tag constants (UTF-8 text)
//   0x05  META      — key=value metadata (name, version, target triple)
//
// The .libx format is intentionally simple:
//   - No compression (object files already compressed by LLVM)
//   - No dependency resolution yet (that's the package manager's job)
//   - Append-only: multiple OBJECT sections = multiple compilation units

use std::fs;
use std::io;
use std::path::Path;

const MAGIC:   &[u8; 4] = b"LIBX";
const VERSION: u32 = 1;

const TAG_OBJECT:  u32 = 0x01;
const TAG_SIGS:    u32 = 0x02;
const TAG_STRUCTS: u32 = 0x03;
const TAG_ENUMS:   u32 = 0x04;
const TAG_META:    u32 = 0x05;

// ─── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum LibxError {
    Io(io::Error),
    BadMagic,
    UnsupportedVersion(u32),
    Corrupt(String),
}

impl std::fmt::Display for LibxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e)              => write!(f, "I/O error: {e}"),
            Self::BadMagic           => write!(f, "not a .libx file (bad magic bytes)"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported .libx version {v}"),
            Self::Corrupt(msg)       => write!(f, "corrupt .libx: {msg}"),
        }
    }
}

impl From<io::Error> for LibxError { fn from(e: io::Error) -> Self { Self::Io(e) } }

// ─── Exported function signature ─────────────────────────────────────────────

/// A single exported function signature stored in the .libx SIGS section.
/// Format: `fn_name(param_type,...) -> ret_type`
#[derive(Debug, Clone)]
pub struct FnSig {
    pub name:   String,
    pub params: Vec<String>,  // type names
    pub ret:    String,
}

impl FnSig {
    pub fn to_sig_line(&self) -> String {
        format!("{}({}) -> {}", self.name, self.params.join(","), self.ret)
    }

    pub fn from_sig_line(line: &str) -> Option<Self> {
        // Parse: `name(p1,p2,...) -> ret`
        let (before_paren, after) = line.split_once('(')?;
        let name = before_paren.trim().to_string();
        let (params_str, ret_part) = after.split_once(')')?;
        let ret = ret_part.trim()
            .trim_start_matches("->")
            .trim()
            .to_string();
        let params = if params_str.trim().is_empty() {
            vec![]
        } else {
            params_str.split(',').map(|s| s.trim().to_string()).collect()
        };
        Some(FnSig { name, params, ret })
    }
}

// ─── LibxWriter ──────────────────────────────────────────────────────────────

pub struct LibxWriter {
    name:    String,
    objects: Vec<Vec<u8>>,
    sigs:    Vec<FnSig>,
    structs: Vec<String>,  // "StructName:field:type,field:type,..."
    enums:   Vec<String>,  // "EnumName:Variant=0,Variant=1,..."
    target:  String,
}

impl LibxWriter {
    pub fn new(name: &str, target: &str) -> Self {
        Self {
            name:    name.to_string(),
            objects: Vec::new(),
            sigs:    Vec::new(),
            structs: Vec::new(),
            enums:   Vec::new(),
            target:  target.to_string(),
        }
    }

    pub fn add_object(&mut self, obj: Vec<u8>) {
        self.objects.push(obj);
    }

    pub fn add_object_file(&mut self, path: &Path) -> Result<(), LibxError> {
        let data = fs::read(path)?;
        self.objects.push(data);
        Ok(())
    }

    pub fn add_sig(&mut self, sig: FnSig) {
        self.sigs.push(sig);
    }

    pub fn add_struct(&mut self, name: &str, fields: &[(&str, &str)]) {
        let fields_str: Vec<String> = fields.iter()
            .map(|(n, t)| format!("{n}:{t}"))
            .collect();
        self.structs.push(format!("{name}:{}", fields_str.join(",")));
    }

    pub fn add_enum(&mut self, name: &str, variants: &[(&str, u64)]) {
        let vars_str: Vec<String> = variants.iter()
            .map(|(n, tag)| format!("{n}={tag}"))
            .collect();
        self.enums.push(format!("{name}:{}", vars_str.join(",")));
    }

    /// Write the complete .libx file to `path`.
    pub fn write(&self, path: &Path) -> Result<(), LibxError> {
        let mut buf = Vec::new();

        // Header
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&VERSION.to_le_bytes());
        write_str_section(&mut buf, &self.name);

        // META section
        let meta = format!(
            "name={}\ntarget={}\nversion={VERSION}\nobjects={}\n",
            self.name, self.target, self.objects.len()
        );
        write_section(&mut buf, TAG_META, meta.as_bytes());

        // OBJECT sections (one per compilation unit)
        for obj in &self.objects {
            write_section(&mut buf, TAG_OBJECT, obj);
        }

        // SIGS section
        if !self.sigs.is_empty() {
            let sigs_text = self.sigs.iter()
                .map(|s| s.to_sig_line())
                .collect::<Vec<_>>()
                .join("\n");
            write_section(&mut buf, TAG_SIGS, sigs_text.as_bytes());
        }

        // STRUCTS section
        if !self.structs.is_empty() {
            let text = self.structs.join("\n");
            write_section(&mut buf, TAG_STRUCTS, text.as_bytes());
        }

        // ENUMS section
        if !self.enums.is_empty() {
            let text = self.enums.join("\n");
            write_section(&mut buf, TAG_ENUMS, text.as_bytes());
        }

        fs::write(path, &buf)?;
        Ok(())
    }
}

// ─── LibxReader ──────────────────────────────────────────────────────────────

pub struct LibxReader {
    pub name:    String,
    pub target:  String,
    pub version: u32,
    pub objects: Vec<Vec<u8>>,
    pub sigs:    Vec<FnSig>,
    pub structs: Vec<String>,
    pub enums:   Vec<String>,
}

impl LibxReader {
    pub fn load(path: &Path) -> Result<Self, LibxError> {
        let data = fs::read(path)?;
        Self::parse(&data)
    }

    pub fn parse(data: &[u8]) -> Result<Self, LibxError> {
        if data.len() < 8 { return Err(LibxError::BadMagic); }
        if &data[0..4] != MAGIC { return Err(LibxError::BadMagic); }

        let version = u32::from_le_bytes(data[4..8].try_into().unwrap());
        if version != VERSION { return Err(LibxError::UnsupportedVersion(version)); }

        // Read library name
        let name_len = u32::from_le_bytes(
            data[8..12].try_into().map_err(|_| LibxError::Corrupt("name_len".into()))?
        ) as usize;
        if data.len() < 12 + name_len {
            return Err(LibxError::Corrupt("truncated name".into()));
        }
        let name = String::from_utf8_lossy(&data[12..12 + name_len]).to_string();

        let mut reader = Self {
            name, version, target: String::new(),
            objects: Vec::new(), sigs: Vec::new(),
            structs: Vec::new(), enums: Vec::new(),
        };

        // Walk sections
        let mut pos = 12 + name_len;
        while pos + 8 <= data.len() {
            let tag = u32::from_le_bytes(data[pos..pos+4].try_into().unwrap());
            let len = u32::from_le_bytes(data[pos+4..pos+8].try_into().unwrap()) as usize;
            pos += 8;

            if pos + len > data.len() {
                return Err(LibxError::Corrupt("section extends past EOF".into()));
            }
            let section_data = &data[pos..pos + len];
            pos += len;

            match tag {
                TAG_OBJECT => reader.objects.push(section_data.to_vec()),
                TAG_SIGS   => {
                    let text = String::from_utf8_lossy(section_data);
                    for line in text.lines() {
                        if let Some(sig) = FnSig::from_sig_line(line) {
                            reader.sigs.push(sig);
                        }
                    }
                }
                TAG_STRUCTS => {
                    let text = String::from_utf8_lossy(section_data);
                    reader.structs = text.lines().map(|l| l.to_string()).collect();
                }
                TAG_ENUMS => {
                    let text = String::from_utf8_lossy(section_data);
                    reader.enums = text.lines().map(|l| l.to_string()).collect();
                }
                TAG_META => {
                    let text = String::from_utf8_lossy(section_data);
                    for line in text.lines() {
                        if let Some(val) = line.strip_prefix("target=") {
                            reader.target = val.to_string();
                        }
                    }
                }
                _ => {} // unknown section — skip
            }
        }

        Ok(reader)
    }

    /// Extract all object files to a directory.
    pub fn extract_objects(&self, dir: &Path) -> Result<Vec<std::path::PathBuf>, LibxError> {
        fs::create_dir_all(dir)?;
        let mut paths = Vec::new();
        for (i, obj) in self.objects.iter().enumerate() {
            let path = dir.join(format!("{}.{i}.o", self.name));
            fs::write(&path, obj)?;
            paths.push(path);
        }
        Ok(paths)
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn write_section(buf: &mut Vec<u8>, tag: u32, data: &[u8]) {
    buf.extend_from_slice(&tag.to_le_bytes());
    buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
    buf.extend_from_slice(data);
}

fn write_str_section(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn roundtrip_empty_lib() {
        let mut w = LibxWriter::new("mylib", "x86_64-unknown-linux-gnu");
        let tmp = std::env::temp_dir().join("test_empty.libx");
        w.write(&tmp).unwrap();
        let r = LibxReader::load(&tmp).unwrap();
        assert_eq!(r.name, "mylib");
        assert_eq!(r.version, 1);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn roundtrip_with_sigs() {
        let mut w = LibxWriter::new("mathlib", "x86_64-unknown-linux-gnu");
        w.add_sig(FnSig {
            name: "add".into(),
            params: vec!["i32".into(), "i32".into()],
            ret: "i32".into(),
        });
        w.add_sig(FnSig {
            name: "factorial".into(),
            params: vec!["i64".into()],
            ret: "i64".into(),
        });
        let tmp = std::env::temp_dir().join("test_sigs.libx");
        w.write(&tmp).unwrap();

        let r = LibxReader::load(&tmp).unwrap();
        assert_eq!(r.sigs.len(), 2);
        assert_eq!(r.sigs[0].name, "add");
        assert_eq!(r.sigs[1].name, "factorial");
        assert_eq!(r.sigs[0].ret, "i32");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn roundtrip_with_struct() {
        let mut w = LibxWriter::new("types", "x86_64-unknown-linux-gnu");
        w.add_struct("Point", &[("x", "f32"), ("y", "f32")]);
        w.add_enum("Color", &[("Red", 0), ("Green", 1), ("Blue", 2)]);
        let tmp = std::env::temp_dir().join("test_struct.libx");
        w.write(&tmp).unwrap();

        let r = LibxReader::load(&tmp).unwrap();
        assert_eq!(r.structs.len(), 1);
        assert!(r.structs[0].contains("Point"));
        assert_eq!(r.enums.len(), 1);
        assert!(r.enums[0].contains("Color"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn bad_magic_rejected() {
        let data = b"NOPE\x01\x00\x00\x00\x00\x00\x00\x00";
        assert!(matches!(LibxReader::parse(data), Err(LibxError::BadMagic)));
    }

    #[test]
    fn sig_line_roundtrip() {
        let sig = FnSig {
            name: "foo".into(),
            params: vec!["i32".into(), "bool".into()],
            ret: "void".into(),
        };
        let line = sig.to_sig_line();
        let parsed = FnSig::from_sig_line(&line).unwrap();
        assert_eq!(parsed.name, "foo");
        assert_eq!(parsed.params, vec!["i32", "bool"]);
        assert_eq!(parsed.ret, "void");
    }
}