use crate::config::{filenamify, types::SrcMountEntry};

#[derive(Debug, Clone)]
pub struct SourcedPath {
    pub origin_slug: String,
    pub local_path: std::path::PathBuf,
    pub protocol: Option<String>,
    pub host: Option<String>,
    pub remote_path: Option<String>,
}

pub fn origin_slug_remote(entry: &SrcMountEntry) -> String {
    let path_part = entry
        .path
        .trim_matches('/')
        .replace('/', "_");
    let path_part = if path_part.is_empty() {
        "root".to_string()
    } else {
        path_part
    };
    filenamify(&format!("sshfs_{}_{}", entry.host, path_part))
}

pub fn origin_slug_local(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    let part = if trimmed.is_empty() {
        "root".to_string()
    } else {
        trimmed.replace('/', "_")
    };
    filenamify(&format!("local_{part}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::SrcMountEntry;

    #[test]
    fn remote_slug_encodes_host_and_path() {
        let entry = SrcMountEntry {
            host: "host1".into(),
            port: 22,
            user: "backup".into(),
            path: "/".into(),
            identity_file: None,
            staging_dir: None,
            ssh_options: vec![],
        };
        assert_eq!(origin_slug_remote(&entry), "sshfs_host1_root");
    }

    #[test]
    fn local_slug_for_etc() {
        assert_eq!(origin_slug_local("/etc"), "local_etc");
    }

    #[test]
    fn distinct_slugs_for_two_hosts() {
        let a = SrcMountEntry {
            host: "host1".into(),
            port: 22,
            user: "u".into(),
            path: "/".into(),
            identity_file: None,
            staging_dir: None,
            ssh_options: vec![],
        };
        let b = SrcMountEntry {
            host: "host2".into(),
            ..a.clone()
        };
        assert_ne!(origin_slug_remote(&a), origin_slug_remote(&b));
    }
}
