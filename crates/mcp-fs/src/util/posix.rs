//! POSIX-style path helpers. The volume tree always uses '/', matching Python's
//! `posixpath` and the C# `Storage/PosixPath.cs` port.

pub struct PosixPath;

impl PosixPath {
    /// Parent of a path, or `None` for the root.
    /// `/a/b/c` -> `/a/b`, `/a` -> `/`, `/` -> None.
    pub fn parent_of(path: &str) -> Option<String> {
        if path == "/" {
            return None;
        }
        let d = Self::dirname(path);
        Some(if d.is_empty() { "/".to_string() } else { d })
    }

    /// Basename, empty for the root.
    pub fn name_of(path: &str) -> String {
        if path == "/" { String::new() } else { Self::basename(path) }
    }

    pub fn dirname(path: &str) -> String {
        match path.rfind('/') {
            None => String::new(),
            Some(0) => "/".to_string(),
            Some(i) => path[..i].to_string(),
        }
    }

    pub fn basename(path: &str) -> String {
        match path.rfind('/') {
            None => path.to_string(),
            Some(i) => path[i + 1..].to_string(),
        }
    }

    /// Equivalent of `posixpath.normpath` for in-volume paths.
    /// For absolute paths a leading `..` is dropped (cannot escape the root).
    pub fn normpath(path: &str) -> String {
        if path.is_empty() {
            return ".".to_string();
        }
        let is_abs = path.starts_with('/');
        let mut stack: Vec<&str> = Vec::new();
        for part in path.split('/') {
            match part {
                "" | "." => continue,
                ".." => {
                    if stack.last().is_some_and(|l| *l != "..") {
                        stack.pop();
                    } else if !is_abs {
                        stack.push("..");
                    }
                    // absolute: '..' at root is dropped, like posixpath
                }
                p => stack.push(p),
            }
        }
        let joined = stack.join("/");
        if is_abs {
            format!("/{joined}")
        } else if joined.is_empty() {
            ".".to_string()
        } else {
            joined
        }
    }

    /// Join a base directory and a relative segment, then normalize.
    pub fn join(base: &str, rel: &str) -> String {
        let b = base.trim_end_matches('/');
        Self::normpath(&format!("{b}/{rel}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_of_cases() {
        assert_eq!(PosixPath::parent_of("/a/b/c").as_deref(), Some("/a/b"));
        assert_eq!(PosixPath::parent_of("/a").as_deref(), Some("/"));
        assert_eq!(PosixPath::parent_of("/"), None);
    }

    #[test]
    fn name_and_basename() {
        assert_eq!(PosixPath::name_of("/a/b.txt"), "b.txt");
        assert_eq!(PosixPath::name_of("/"), "");
        assert_eq!(PosixPath::basename("/a/b.txt"), "b.txt");
        assert_eq!(PosixPath::basename("noslash"), "noslash");
    }

    #[test]
    fn dirname_cases() {
        assert_eq!(PosixPath::dirname("/a/b/c"), "/a/b");
        assert_eq!(PosixPath::dirname("/a"), "/");
        assert_eq!(PosixPath::dirname("noslash"), "");
    }

    /// Same table as the C# PosixPathTests.
    #[test]
    fn normpath_matches_csharp_table() {
        assert_eq!(PosixPath::normpath("/a/../b"), "/b");
        assert_eq!(PosixPath::normpath("/a/./b/"), "/a/b");
        assert_eq!(PosixPath::normpath("/../etc"), "/etc");
        assert_eq!(PosixPath::normpath("/"), "/");
        assert_eq!(PosixPath::normpath(""), ".");
        assert_eq!(PosixPath::normpath("a/b"), "a/b");
        assert_eq!(PosixPath::normpath("./a"), "a");
    }

    #[test]
    fn normpath_absolute_cannot_escape_root() {
        assert_eq!(PosixPath::normpath("/../../etc/passwd"), "/etc/passwd");
        assert_eq!(PosixPath::normpath("/a/../../b"), "/b");
    }

    #[test]
    fn normpath_relative_keeps_leading_dotdot() {
        assert_eq!(PosixPath::normpath("../a"), "../a");
        assert_eq!(PosixPath::normpath("../../a"), "../../a");
    }

    #[test]
    fn normpath_collapses_repeated_slashes() {
        assert_eq!(PosixPath::normpath("/a//b///c"), "/a/b/c");
    }

    #[test]
    fn join_normalizes() {
        assert_eq!(PosixPath::join("/a", "b.txt"), "/a/b.txt");
        assert_eq!(PosixPath::join("/a/", "b.txt"), "/a/b.txt");
        assert_eq!(PosixPath::join("/a", "../b.txt"), "/b.txt");
        assert_eq!(PosixPath::join("/", "x"), "/x");
    }
}
