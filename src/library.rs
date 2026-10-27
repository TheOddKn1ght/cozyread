use std::{
    cmp::Ordering,
    fs, io,
    path::{Path, PathBuf},
};

pub fn scan(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect(root, &mut files)?;
    files.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    Ok(files)
}

fn collect(path: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if path.is_file() {
        if supported(path) {
            files.push(path.canonicalize()?);
        }
    } else if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_symlink() {
                continue;
            }
            collect(&entry.path(), files)?;
        }
    } else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Library path does not exist",
        ));
    }
    Ok(())
}

fn supported(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "mp3" | "m4a" | "m4b" | "mp4" | "flac" | "ogg" | "oga" | "wav"
        )
    })
}

fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let order = if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (start_i, start_j) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let x = a[start_i..i]
                .iter()
                .position(|c| *c != b'0')
                .map_or(&a[i..i], |n| &a[start_i + n..i]);
            let y = b[start_j..j]
                .iter()
                .position(|c| *c != b'0')
                .map_or(&b[j..j], |n| &b[start_j + n..j]);
            x.len().cmp(&y.len()).then_with(|| x.cmp(y))
        } else {
            let order = a[i].cmp(&b[j]);
            i += 1;
            j += 1;
            order
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chapters_sort_numerically_without_integer_overflow() {
        let mut names = [
            "chapter 10",
            "chapter 2",
            "chapter 1",
            "chapter 999999999999999999999999",
        ];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names[0..3], ["chapter 1", "chapter 2", "chapter 10"]);
    }
    #[test]
    fn extensions_are_case_insensitive() {
        assert!(supported(Path::new("Book.M4B")));
        assert!(!supported(Path::new("cover.jpg")));
    }
}
