//! Machine profile files (S24): `<data_dir>/machines/<id>.json`.

use std::fs;
use std::path::{Path, PathBuf};

use openlaser_machine::{MachineId, MachineProfile, ProfileStore, jpt_m7_200, jpt_m7_200_sim, om_gantries, validate_profile};

/// Returns the per-user OpenLaser data directory when the platform can provide one.
pub fn profiles_dir() -> Option<PathBuf> {
    craft_settings::paths::data_dir("OpenLaser").map(|dir| dir.join("machines"))
}

/// Loads valid saved profiles over the built-ins. Invalid files are skipped and reported.
pub fn load_all(dir: Option<&Path>) -> (Vec<MachineProfile>, Vec<String>) {
    let mut profiles = vec![jpt_m7_200(), jpt_m7_200_sim()];
    profiles.extend(om_gantries());
    let Some(dir) = dir else {
        return (profiles, vec!["machine profile directory is unavailable".to_owned()]);
    };
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (profiles, Vec::new()),
        Err(error) => return (profiles, vec![format!("cannot read machine profile directory: {error}")]),
    };
    let mut paths = entries
        .filter_map(|entry| match entry {
            Ok(entry) if entry.path().extension().is_some_and(|ext| ext == "json") => Some(Ok(entry.path())),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Vec<_>>();
    paths.sort_by(|a, b| a.as_ref().ok().cmp(&b.as_ref().ok()));
    let mut messages = Vec::new();
    for path in paths {
        let path = match path {
            Ok(path) => path,
            Err(error) => {
                messages.push(format!("cannot enumerate machine profiles: {error}"));
                continue;
            }
        };
        let loaded = fs::read(&path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| serde_json::from_slice::<MachineProfile>(&bytes).map_err(|error| error.to_string()))
            .and_then(|profile| {
                validate_id(&profile.id.0)?;
                let stem = path.file_stem().and_then(|stem| stem.to_str()).ok_or_else(|| "profile filename is not valid UTF-8".to_owned())?;
                if stem != profile.id.0 {
                    return Err(format!("profile file stem `{stem}` does not match id `{}`", profile.id.0));
                }
                validate_profile(&profile).map(|()| profile).map_err(|errors| errors.join("; "))
            });
        match loaded {
            Ok(profile) => {
                if let Some(existing) = profiles.iter_mut().find(|item| item.id == profile.id) {
                    *existing = profile;
                } else {
                    profiles.push(profile);
                }
            }
            Err(error) => messages.push(format!("skipped invalid machine profile {}: {error}", path.display())),
        }
    }
    (profiles, messages)
}

/// Atomic profile-file store used only by human surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirProfileStore {
    dir: PathBuf,
}

impl DirProfileStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl ProfileStore for DirProfileStore {
    fn save(&self, profile: &MachineProfile) -> Result<(), String> {
        validate_profile(profile).map_err(|errors| errors.join("; "))?;
        validate_id(&profile.id.0)?;
        fs::create_dir_all(&self.dir).map_err(|error| error.to_string())?;
        let bytes = serde_json::to_vec_pretty(profile).map_err(|error| error.to_string())?;
        craft_atomic::atomic_write(&self.dir.join(format!("{}.json", profile.id.0)), &bytes).map_err(|error| error.to_string())
    }

    fn delete(&self, id: &MachineId) -> Result<(), String> {
        validate_id(&id.0)?;
        match fs::remove_file(self.dir.join(format!("{}.json", id.0))) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(format!("machine profile {} does not exist", id.0)),
            Err(error) => Err(error.to_string()),
        }
    }
}

fn validate_id(id: &str) -> Result<(), String> {
    let lowercase = id.to_ascii_lowercase();
    let reserved = matches!(lowercase.as_str(), "con" | "nul" | "aux" | "prn")
        || ["com", "lpt"].iter().any(|prefix| (1..=9).any(|number| lowercase == format!("{prefix}{number}")));
    if !reserved && (1..=64).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-') {
        Ok(())
    } else {
        Err("machine profile id must match [a-z0-9-]{1,64}".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_profile_shadows_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let store = DirProfileStore::new(dir.path().to_path_buf());
        let mut profile = jpt_m7_200_sim();
        profile.name = "Saved simulator".to_owned();
        store.save(&profile).unwrap();
        let (all, messages) = load_all(Some(dir.path()));
        assert!(messages.is_empty());
        assert_eq!(all.iter().filter(|item| item.id == profile.id).count(), 1);
        assert_eq!(all.iter().find(|item| item.id == profile.id).map(|item| item.name.as_str()), Some("Saved simulator"));
    }

    #[test]
    fn invalid_profile_file_is_skipped_with_message() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("bad.json"), b"{}").unwrap();
        let (all, messages) = load_all(Some(dir.path()));
        assert_eq!(all.len(), 5);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("bad.json"));
    }

    #[test]
    fn profile_loader_checks_id_filename_and_windows_reserved_names() {
        for reserved in ["con", "nul", "aux", "prn", "com1", "com9", "lpt1", "lpt9"] {
            assert!(validate_id(reserved).is_err(), "accepted reserved id {reserved}");
        }
        let dir = tempfile::tempdir().unwrap();
        let profile = jpt_m7_200_sim();
        let bytes = serde_json::to_vec(&profile).unwrap();
        fs::write(dir.path().join("other.json"), bytes).unwrap();
        let (_, messages) = load_all(Some(dir.path()));
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("does not match id"));
    }
}
