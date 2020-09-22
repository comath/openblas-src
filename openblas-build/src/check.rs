//! Check make results

use super::*;
use anyhow::Result;
use std::{
    collections::HashSet,
    fs,
    hash::Hash,
    io::{self, BufRead},
    path::*,
};

/// Parse compiler linker flags, `-L` and `-l`
///
/// - Search paths defined by `-L` will be removed if not exists,
///   and will be canonicalize
///
/// ```
/// use openblas_build::*;
/// let info = LinkInfo::parse("-L/usr/lib/gcc/x86_64-pc-linux-gnu/10.2.0 -L/usr/lib/gcc/x86_64-pc-linux-gnu/10.2.0/../../../../lib -L/lib/../lib -L/usr/lib/../lib -L/usr/lib/gcc/x86_64-pc-linux-gnu/10.2.0/../../..  -lc");
/// assert_eq!(info.libs, vec!["c"]);
/// ```
#[derive(Debug, Clone, Default)]
pub struct LinkInfo {
    pub search_paths: Vec<PathBuf>,
    pub libs: Vec<String>,
}

fn as_sorted_vec<T: Hash + Ord>(set: HashSet<T>) -> Vec<T> {
    let mut v: Vec<_> = set.into_iter().collect();
    v.sort();
    v
}

impl LinkInfo {
    pub fn parse(line: &str) -> Self {
        let mut search_paths = HashSet::new();
        let mut libs = HashSet::new();
        for entry in line.split(" ") {
            if entry.starts_with("-L") {
                let path = PathBuf::from(entry.trim_start_matches("-L"));
                if !path.exists() {
                    continue;
                }
                search_paths.insert(path.canonicalize().expect("Failed to canonicalize path"));
            }
            if entry.starts_with("-l") {
                libs.insert(entry.trim_start_matches("-l").into());
            }
        }
        LinkInfo {
            search_paths: as_sorted_vec(search_paths),
            libs: as_sorted_vec(libs),
        }
    }
}

/// Parse Makefile.conf which generated by OpenBLAS make system
#[derive(Debug, Clone, Default)]
pub struct MakeConf {
    os_name: String,
    no_fortran: bool,
    c_extra_libs: LinkInfo,
    f_extra_libs: LinkInfo,
}

impl MakeConf {
    /// Parse from file
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut detail = MakeConf::default();
        let f = fs::File::open(path)?;
        let buf = io::BufReader::new(f);
        for line in buf.lines() {
            let line = line.unwrap();
            if line.len() == 0 {
                continue;
            }
            let entry: Vec<_> = line.split("=").collect();
            if entry.len() != 2 {
                continue;
            }
            match entry[0] {
                "OSNAME" => detail.os_name = entry[1].into(),
                "NOFORTRAN" => detail.no_fortran = true,
                "CEXTRALIB" => detail.c_extra_libs = LinkInfo::parse(entry[1]),
                "FEXTRALIB" => detail.f_extra_libs = LinkInfo::parse(entry[1]),
                _ => continue,
            }
        }
        Ok(detail)
    }
}

#[derive(Debug, Clone)]
pub struct LibDetail {
    /// File path of library
    path: PathBuf,

    /// Linked shared libraries. It will be empty if the library is static.
    /// Use `objdump -p` external command.
    libs: Vec<String>,

    /// Global "T" symbols in the text (code) section of library.
    /// Use `nm -g` external command.
    symbols: Vec<String>,
}

impl LibDetail {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        let path = path.as_ref();
        if !path.exists() {
            panic!("File not found: {}", path.display());
        }

        let nm_out = Command::new("nm")
            .arg("-g")
            .arg(path)
            .output()
            .expect("nm cannot be started");

        // assumes `nm` output like following:
        //
        // ```
        // 0000000000909b30 T zupmtr_
        // ```
        let mut symbols: Vec<_> = nm_out
            .stdout
            .lines()
            .flat_map(|line| {
                let line = line.ok()?;
                let entry: Vec<_> = line.trim().split(" ").collect();
                if entry.len() != 3 && entry[2] == "T" {
                    None
                } else {
                    Some(entry[2].into())
                }
            })
            .collect();
        symbols.sort(); // sort alphabetically

        let mut libs: Vec<_> = Command::new("objdump")
            .arg("-p")
            .arg(path)
            .output()
            .expect("objdump cannot start")
            .stdout
            .lines()
            .flat_map(|line| {
                let line = line.ok()?;
                if line.trim().starts_with("NEEDED") {
                    Some(line.trim().trim_start_matches("NEEDED").trim().into())
                } else {
                    None
                }
            })
            .collect();
        libs.sort();

        LibDetail {
            path: path.into(),
            libs,
            symbols,
        }
    }

    pub fn has_cblas(&self) -> bool {
        for sym in &self.symbols {
            if sym.starts_with("cblas_") {
                return true;
            }
        }
        return false;
    }

    pub fn has_lapack(&self) -> bool {
        for sym in &self.symbols {
            if sym == "dsyev_" {
                return true;
            }
        }
        return false;
    }

    pub fn has_lapacke(&self) -> bool {
        for sym in &self.symbols {
            if sym.starts_with("LAPACKE_") {
                return true;
            }
        }
        return false;
    }

    pub fn has_lib(&self, name: &str) -> bool {
        for lib in &self.libs {
            if let Some(stem) = lib.split(".").next() {
                if stem == format!("lib{}", name) {
                    return true;
                }
            };
        }
        return false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_from_makefile_conf() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Makefile.conf");
        assert!(path.exists());
        let detail = MakeConf::new(path).unwrap();
        assert!(!detail.no_fortran);
    }

    #[test]
    fn detail_from_nofortran_conf() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("nofortran.conf");
        assert!(path.exists());
        let detail = MakeConf::new(path).unwrap();
        assert!(detail.no_fortran);
    }
}
