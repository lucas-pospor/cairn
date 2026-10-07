//! Whether an entry fits in the Recycle Bin of its drive. The shell asks
//! nothing before it deletes for good an entry larger than the bin takes,
//! and `recycle_bin`'s progress sink only learns of it once the entry is
//! gone, so such an entry never gets to the shell. The numbers are worked
//! out here, apart from where Windows keeps them, so that the tests run on
//! every system, and so is what a delete that the shell was given came to
//! (`verdict`).

use std::path::Path;

use crate::error::os_text;
#[cfg(feature = "system-trash")]
use crate::fs::SystemTrash;

/// The bytes in a megabyte of MaxCapacity, and in a gigabyte of the default
/// size. Windows probably counts 2^20 and 2^30 bytes; a million and a
/// billion give a smaller maximum, so an entry close to the real one goes
/// to the notebook's .trash rather than to a bin that may not take it.
const MB: u64 = 1_000_000;
const GB: u64 = 1_000_000_000;

/// The Recycle Bin settings of a drive, as Windows keeps them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BinSettings {
    /// The drive's own maximum, in megabytes, as Recycle Bin, Properties
    /// sets it (MaxCapacity), if it has one.
    pub max_capacity: Option<u32>,
    /// The "Maximum allowed Recycle Bin size" policy (RecycleBinSize), in
    /// percent of the drive, if it is set.
    pub size_policy: Option<u32>,
    /// The bytes on the drive that the user may use: its size, or the
    /// user's quota on it.
    pub drive_size: u64,
}

impl BinSettings {
    /// The most bytes the bin takes. The policy holds for every drive, and
    /// users cannot change the size while it is set (Microsoft's Policy CSP
    /// documentation, ADMX_WindowsExplorer/RecycleBinSize); where a drive
    /// also has its own maximum, the smaller of the two counts. Without
    /// either, Windows gives the bin ten percent of the first 40 GB of the
    /// user's quota on the drive and five percent of the rest (Raymond
    /// Chen, "What is the default size of the Recycle Bin, and how can an
    /// administrator control the size of the Recycle Bin?", The Old New
    /// Thing, 2014-07-01).
    pub fn maximum(&self) -> u64 {
        let share = |percent: u32| u64::try_from(u128::from(self.drive_size) * u128::from(percent) / 100).unwrap_or(u64::MAX);
        let set = [self.max_capacity.map(|mb| u64::from(mb) * MB), self.size_policy.map(share)];
        set.into_iter().flatten().min().unwrap_or_else(|| {
            let first = self.drive_size.min(40 * GB);
            first / 10 + (self.drive_size - first) / 20
        })
    }
}

/// Whether the entry at `path` fits in the Recycle Bin that `bin` describes,
/// or why not, for the log. Settings that cannot be read, and an entry
/// whose size cannot be, count as not fitting: the entry then goes to the
/// notebook's .trash instead.
pub(crate) fn fits(path: &Path, bin: Result<BinSettings, String>) -> Result<(), String> {
    let max = bin.map_err(|e| format!("the size of the Recycle Bin of its drive cannot be read: {e}"))?.maximum();
    let size = size_up_to(path, max).map_err(|e| format!("its size cannot be read: {}", os_text(&e)))?;
    if size > max {
        return Err(format!("it is too large for the Recycle Bin of its drive, which takes up to {}", megabytes(max)));
    }
    Ok(())
}

/// The bytes of the entry at `path`: the length of a file, or the lengths
/// of all the files in a folder and in its folders. A symlink or junction
/// counts as nothing and is not followed, also when it is the entry itself:
/// the bin takes the link, not what it leads to. The count stops once it is
/// over `limit`.
pub(crate) fn size_up_to(path: &Path, limit: u64) -> std::io::Result<u64> {
    let md = std::fs::symlink_metadata(path)?;
    if !md.is_dir() {
        return Ok(if md.file_type().is_symlink() { 0 } else { md.len() });
    }
    let (mut total, mut dirs) = (0u64, vec![path.to_path_buf()]);
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            // The entry's own metadata: a link in the folder is not followed.
            let md = entry.metadata()?;
            if md.is_dir() {
                dirs.push(entry.path());
            } else if !md.file_type().is_symlink() {
                total = total.saturating_add(md.len());
                if total > limit {
                    return Ok(total);
                }
            }
        }
    }
    Ok(total)
}

/// What `recycle_bin`'s progress sink saw during the shell's delete of an
/// entry.
#[cfg(feature = "system-trash")]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Seen {
    /// The shell was about to delete something for good, and was stopped.
    pub refused: bool,
    /// The shell deleted something for good all the same.
    pub destroyed: bool,
    /// The shell put something in the Recycle Bin.
    pub kept: bool,
}

/// What became of the entry at `path` that the shell was given to delete:
/// what the sink saw, whether the entry is `gone`, and what the operation
/// reported (`aborted`: whether the shell stopped some of it, or why it
/// failed). An entry that is gone, with a delete for good and nothing put
/// in the Recycle Bin, is `Destroyed`: nothing brings it back, and the
/// delete must not look as if it had worked (WIN-005). A delete for good
/// next to an entry still there, or to one recycled, is logged.
#[cfg(feature = "system-trash")]
pub(crate) fn verdict(path: &Path, seen: Seen, gone: bool, aborted: Result<bool, String>) -> SystemTrash {
    if seen.destroyed && !seen.kept && gone {
        return SystemTrash::Destroyed;
    }
    if seen.destroyed {
        log::warn!("the shell reported deleting something for good while it took {}", path.display());
    }
    if seen.refused {
        return SystemTrash::NotRecyclable(
            "the shell would delete it for good (no Recycle Bin on its drive, the bin set to remove files at once, or too large for the bin)".into(),
        );
    }
    match aborted {
        Ok(false) => SystemTrash::Recycled,
        Ok(true) => SystemTrash::Failed("the shell stopped the delete".into()),
        Err(e) => SystemTrash::Failed(e),
    }
}

/// `bytes` in the megabytes of MaxCapacity, for the log.
fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / MB as f64;
    if mb >= 10.0 { format!("{mb:.0} MB") } else { format!("{mb:.1} MB") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bin(max_capacity: Option<u32>, size_policy: Option<u32>, drive_size: u64) -> BinSettings {
        BinSettings { max_capacity, size_policy, drive_size }
    }

    /// The drive's own maximum is in megabytes, the policy is a share of
    /// the drive, and where both are set the smaller counts. Without
    /// either, the bin gets ten percent of the first 40 GB and five percent
    /// of the rest.
    #[test]
    fn the_bin_takes_what_its_settings_say() {
        let drive = 500 * GB;
        assert_eq!(bin(Some(1), None, drive).maximum(), MB);
        assert_eq!(bin(Some(14209), None, drive).maximum(), 14209 * MB);
        assert_eq!(bin(None, Some(10), drive).maximum(), 50 * GB);
        assert_eq!(bin(None, Some(0), drive).maximum(), 0);
        assert_eq!(bin(Some(1), Some(10), drive).maximum(), MB);
        assert_eq!(bin(Some(100_000), Some(10), drive).maximum(), 50 * GB);
        assert_eq!(bin(Some(u32::MAX), None, drive).maximum(), u64::from(u32::MAX) * MB);
        assert_eq!(bin(None, Some(u32::MAX), u64::MAX).maximum(), u64::MAX);
        assert_eq!(bin(None, None, 0).maximum(), 0);
        assert_eq!(bin(None, None, 30 * GB).maximum(), 3 * GB);
        assert_eq!(bin(None, None, 40 * GB).maximum(), 4 * GB);
        assert_eq!(bin(None, None, 240 * GB).maximum(), 4 * GB + 10 * GB);
        assert_eq!(bin(None, None, u64::MAX).maximum(), 4 * GB + (u64::MAX - 40 * GB) / 20);
    }

    /// An entry fits when it is no larger than the bin takes. Settings that
    /// cannot be read, or an entry whose size cannot be, send it to the
    /// notebook's .trash, with the reason for the log.
    #[test]
    fn an_entry_larger_than_the_bin_does_not_fit() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.bin");
        std::fs::write(&f, [7u8; 1000]).unwrap();
        // A drive of 10,000 bytes gets a bin of 1,000 by default.
        assert_eq!(fits(&f, Ok(bin(None, None, 10_000))), Ok(()));
        assert_eq!(fits(&f, Ok(bin(None, Some(11), 10_000))), Ok(()));
        assert_eq!(fits(&f, Ok(bin(None, Some(9), 10_000))), Err("it is too large for the Recycle Bin of its drive, which takes up to 0.0 MB".into()));
        assert_eq!(fits(&f, Ok(bin(None, None, 9_999))).map_err(|e| e.starts_with("it is too large")), Err(true));
        assert_eq!(fits(&f, Ok(bin(Some(1), None, 0))), Ok(()));
        let big = d.path().join("big.bin");
        std::fs::write(&big, vec![7u8; 1_500_000]).unwrap();
        assert_eq!(fits(&big, Ok(bin(Some(1), None, 500 * GB))), Err("it is too large for the Recycle Bin of its drive, which takes up to 1.0 MB".into()));
        assert_eq!(fits(&big, Ok(bin(Some(2), None, 500 * GB))), Ok(()));
        assert_eq!(fits(&big, Ok(bin(Some(14209), Some(0), 500 * GB))).map_err(|e| e.ends_with("up to 0.0 MB")), Err(true));
        // A maximum that cannot be read: an access denied, a value of the
        // wrong type, a drive without a volume GUID.
        let why = fits(&f, Err("MaxCapacity cannot be read: Access is denied.".into()));
        assert_eq!(why, Err("the size of the Recycle Bin of its drive cannot be read: MaxCapacity cannot be read: Access is denied.".into()));
        let gone = fits(&d.path().join("gone.bin"), Ok(bin(None, None, 500 * GB)));
        assert!(gone.as_ref().is_err_and(|e| e.starts_with("its size cannot be read: ")), "{gone:?}");
    }

    /// A delete for good with nothing kept and the entry gone is
    /// `Destroyed`, whatever else the shell reported. A delete for good
    /// next to something recycled, or with the entry still there, is
    /// logged and judged by the rest: a stopped delete for good leaves the
    /// entry for the notebook's .trash, and a stopped or failed operation is
    /// a failure.
    #[cfg(feature = "system-trash")]
    #[test]
    fn what_the_shell_did_with_a_delete() {
        let lines = crate::fs::tests::logged();
        let seen = |refused, destroyed, kept| Seen { refused, destroyed, kept };
        let not_recyclable = || {
            SystemTrash::NotRecyclable(
                "the shell would delete it for good (no Recycle Bin on its drive, the bin set to remove files at once, or too large for the bin)".into(),
            )
        };
        let p = Path::new(r"C:\verdict\a.md");
        let cases = [
            // The size check missed it: gone for good, and nothing kept.
            (seen(false, true, false), true, Ok(false), SystemTrash::Destroyed),
            (seen(true, true, false), true, Ok(true), SystemTrash::Destroyed),
            (seen(false, true, false), true, Err("failed".to_string()), SystemTrash::Destroyed),
            // Something of a folder went to the bin, something did not.
            (seen(false, true, true), true, Ok(false), SystemTrash::Recycled),
            // Reported, but the entry is still there.
            (seen(true, true, false), false, Ok(true), not_recyclable()),
            (seen(false, true, false), false, Ok(false), SystemTrash::Recycled),
            // No delete for good.
            (seen(false, false, false), true, Ok(false), SystemTrash::Recycled),
            (seen(false, false, true), true, Ok(false), SystemTrash::Recycled),
            (seen(true, false, false), false, Ok(true), not_recyclable()),
            (seen(true, false, true), false, Ok(true), not_recyclable()),
            (seen(false, false, false), false, Ok(true), SystemTrash::Failed("the shell stopped the delete".into())),
            (seen(false, false, false), false, Err("Access is denied.".to_string()), SystemTrash::Failed("Access is denied.".into())),
        ];
        for (seen, gone, aborted, want) in cases {
            assert_eq!(verdict(p, seen, gone, aborted.clone()), want, "{seen:?}, gone {gone}, {aborted:?}");
        }
        let warned = lines.lock().iter().filter(|l| l.contains(r"C:\verdict\a.md")).cloned().collect::<Vec<_>>();
        let warning = r"WARN the shell reported deleting something for good while it took C:\verdict\a.md";
        assert_eq!(warned, [warning; 3]);
    }

    /// A folder counts the files in it and in its folders, and a symlink in
    /// it counts as nothing: neither a link to a big file nor one that
    /// loops back to the folder is followed. The count stops once it is
    /// over the limit.
    #[cfg(unix)]
    #[test]
    fn a_folder_counts_its_files_and_not_where_its_links_lead() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        let (f, big) = (d.path().join("f"), d.path().join("big.bin"));
        std::fs::create_dir_all(f.join("sub").join("empty")).unwrap();
        std::fs::write(f.join("a.md"), [b'a'; 600]).unwrap();
        std::fs::write(f.join("sub").join("b.md"), [b'b'; 400]).unwrap();
        std::fs::write(&big, vec![0u8; 100_000]).unwrap();
        symlink(&big, f.join("big.bin")).unwrap();
        symlink(d.path(), f.join("sub").join("outside")).unwrap();
        symlink("..", f.join("sub").join("loop")).unwrap();
        assert_eq!(size_up_to(&f, u64::MAX).unwrap(), 1000);
        assert_eq!(size_up_to(&f.join("big.bin"), u64::MAX).unwrap(), 0);
        assert_eq!(size_up_to(&f.join("sub").join("loop"), u64::MAX).unwrap(), 0);
        assert_eq!(size_up_to(&big, u64::MAX).unwrap(), 100_000);
        assert!(size_up_to(&f, 500).unwrap() > 500);
        // Through `fits`: a bin of 1,000 bytes takes the folder, one of 999 does not.
        assert_eq!(fits(&f, Ok(bin(None, None, 10_000))), Ok(()));
        assert!(fits(&f, Ok(bin(None, None, 9_990))).is_err());
        assert_eq!(fits(&f.join("big.bin"), Ok(bin(None, Some(0), 10_000))), Ok(()));
    }
}
