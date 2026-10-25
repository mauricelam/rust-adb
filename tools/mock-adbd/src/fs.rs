use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub mode: u32,
    pub mtime: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    File(FileEntry),
    Directory {
        mode: u32,
        mtime: u32,
    },
    Symlink {
        target: String,
        mode: u32,
        mtime: u32,
    },
}

impl Node {
    pub fn mode(&self) -> u32 {
        match self {
            Node::File(f) => f.mode,
            Node::Directory { mode, .. } => *mode,
            Node::Symlink { mode, .. } => *mode,
        }
    }

    pub fn mtime(&self) -> u32 {
        match self {
            Node::File(f) => f.mtime,
            Node::Directory { mtime, .. } => *mtime,
            Node::Symlink { mtime, .. } => *mtime,
        }
    }

    pub fn size(&self) -> u64 {
        match self {
            Node::File(f) => f.data.len() as u64,
            Node::Directory { .. } => 0,
            Node::Symlink { target, .. } => target.len() as u64,
        }
    }
}

#[derive(Debug, Clone)]
pub struct InMemoryFs {
    nodes: Arc<Mutex<BTreeMap<String, Node>>>,
}

pub fn normalize_path(path: &str) -> String {
    let mut clean = String::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        clean.push('/');
        clean.push_str(part);
    }
    if clean.is_empty() {
        "/".to_string()
    } else {
        clean
    }
}

impl Default for InMemoryFs {
    fn default() -> Self {
        let fs = Self {
            nodes: Arc::new(Mutex::new(BTreeMap::new())),
        };
        // Root directory '/'
        fs.add_dir("/", 0o040755, 0);
        // Standard test directory '/data'
        fs.add_dir("/data", 0o040755, 1000);
        fs.add_file("/data/f_0.bin", vec![], 0o100644, 2000);
        fs.add_file("/data/f_1.bin", vec![0x42], 0o100644, 2000);
        fs.add_file("/data/f_65535.bin", vec![0xAA; 65535], 0o100644, 2000);
        fs.add_file("/data/f_65536.bin", vec![0xBB; 65536], 0o100644, 2000);
        let f300k: Vec<u8> = (0..307200).map(|i| (i % 256) as u8).collect();
        fs.add_file("/data/f_300k.bin", f300k, 0o100644, 2000);
        fs.add_file("/data/fail.bin", vec![], 0o100644, 2000);
        fs.add_file("/data/disconnect.bin", vec![0xCC; 100000], 0o100644, 2000);
        fs
    }
}

impl InMemoryFs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_dir(&self, path: &str, mode: u32, mtime: u32) {
        let norm = normalize_path(path);
        let mode = if (mode & 0o170000) == 0 {
            mode | 0o040000
        } else {
            mode
        };
        let mut guard = self.nodes.lock().unwrap();
        guard.insert(norm, Node::Directory { mode, mtime });
    }

    pub fn add_file(&self, path: &str, data: Vec<u8>, mode: u32, mtime: u32) {
        let norm = normalize_path(path);
        let mode = if (mode & 0o170000) == 0 {
            mode | 0o100000
        } else {
            mode
        };
        let mut guard = self.nodes.lock().unwrap();
        guard.insert(norm, Node::File(FileEntry { mode, mtime, data }));
    }

    pub fn add_symlink(&self, path: &str, target: &str, mode: u32, mtime: u32) {
        let norm = normalize_path(path);
        let mode = if (mode & 0o170000) == 0 {
            mode | 0o120000
        } else {
            mode
        };
        let mut guard = self.nodes.lock().unwrap();
        guard.insert(
            norm,
            Node::Symlink {
                target: target.to_string(),
                mode,
                mtime,
            },
        );
    }

    pub fn get(&self, path: &str) -> Option<Node> {
        let norm = normalize_path(path);
        let guard = self.nodes.lock().unwrap();
        guard.get(&norm).cloned()
    }

    pub fn list(&self, dir_path: &str) -> Option<Vec<(String, Node)>> {
        let norm = normalize_path(dir_path);
        let guard = self.nodes.lock().unwrap();

        let parent_node = guard.get(&norm)?;
        match parent_node {
            Node::Directory { .. } => {}
            _ => return None,
        }

        let prefix = if norm == "/" {
            "/".to_string()
        } else {
            format!("{norm}/")
        };

        let mut entries = Vec::new();
        for (path, node) in guard.iter() {
            if path == &norm {
                continue;
            }
            if path.starts_with(&prefix) {
                let rest = &path[prefix.len()..];
                if !rest.contains('/') && !rest.is_empty() {
                    entries.push((rest.to_string(), node.clone()));
                }
            }
        }
        Some(entries)
    }

    pub fn parent_exists(&self, path: &str) -> bool {
        let norm = normalize_path(path);
        if norm == "/" {
            return true;
        }
        if let Some(idx) = norm.rfind('/') {
            let parent = if idx == 0 { "/" } else { &norm[..idx] };
            if let Some(Node::Directory { .. }) = self.get(parent) {
                return true;
            }
        }
        false
    }
}
