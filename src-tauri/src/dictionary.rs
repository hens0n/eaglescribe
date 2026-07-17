//! Personal dictionary: force preferred spellings for names, brands, jargon.
//!
//! Stored locally as JSON under the app data directory. Applied after polish
//! with case-insensitive whole-word (or multi-word phrase) matching.

use crate::error::{AppError, AppResult};
use crate::recognition::RecognitionFingerprint;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const DICTIONARY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryOrigin {
    Manual,
    Tuning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryEditState {
    Unmodified,
    ModifiedAfterVerification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedRecognitionFingerprint {
    pub fingerprint: RecognitionFingerprint,
    pub verified_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationConflict {
    pub id: String,
    pub canonical_from: String,
    /// Exact legacy mappings retained for explicit user selection.
    pub choices: Vec<DictEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationConflictResolution {
    pub conflict_id: String,
    pub selected_entry_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictionaryEntryIdentity {
    pub id: String,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuningOverlayApplication {
    pub pre_overlay: String,
    pub post_overlay: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictEntry {
    /// Stable identity used by paused Tuning Sessions and concurrent editors.
    pub id: String,
    /// What STT typically produces (or a phrase to match), e.g. "eagle scribe".
    pub from: String,
    /// Preferred spelling to insert, e.g. "EagleScribe".
    pub to: String,
    pub origin: EntryOrigin,
    pub edit_state: EntryEditState,
    pub verified_fingerprints: Vec<VerifiedRecognitionFingerprint>,
    /// Monotonic identity for optimistic concurrency on this entry.
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dictionary {
    pub schema_version: u32,
    /// Monotonic identity for the complete dictionary snapshot.
    pub revision: u64,
    pub entries: Vec<DictEntry>,
    #[serde(default)]
    pub migration_conflicts: Vec<MigrationConflict>,
}

impl Default for Dictionary {
    fn default() -> Self {
        Self {
            schema_version: DICTIONARY_SCHEMA_VERSION,
            revision: 0,
            entries: Vec::new(),
            migration_conflicts: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct LegacyDictionary {
    #[serde(default)]
    entries: Vec<LegacyDictEntry>,
}

#[derive(Debug, Deserialize)]
struct LegacyDictEntry {
    from: String,
    to: String,
}

impl Dictionary {
    pub(crate) fn entry_for_source(&self, source: &str) -> Option<&DictEntry> {
        let canonical_source = canonical_text(source);
        self.entries
            .iter()
            .find(|entry| canonical_text(&entry.from) == canonical_source)
    }

    /// Load mappings for the running app even when migration persistence fails.
    /// The warning must be surfaced to the user; the readable mappings remain active.
    pub fn load_for_runtime(path: &Path) -> (Self, Option<String>) {
        match Self::load(path) {
            Ok(dictionary) => (dictionary, None),
            Err(error) => (
                Self::read_without_persisting(path).unwrap_or_default(),
                Some(format!("Dictionary storage error: {error}")),
            ),
        }
    }

    pub fn load(path: &Path) -> AppResult<Self> {
        if !path.is_file() {
            if let Some(backup) = read_recovery_backup(path) {
                return parse_dictionary(&backup).map(|(dictionary, _)| dictionary);
            }
            return Ok(Self::default());
        }
        let data = fs::read_to_string(path)
            .map_err(|e| AppError::from(format!("Read dictionary failed: {e}")))?;
        if data.trim().is_empty() {
            if let Some(backup) = read_recovery_backup(path) {
                return parse_dictionary(&backup).map(|(dictionary, _)| dictionary);
            }
            return Ok(Self::default());
        }
        match parse_dictionary(&data) {
            Ok((dictionary, false)) => {
                reconcile_prepared_backup(path, &data);
                Ok(dictionary)
            }
            Ok((dictionary, true)) => {
                dictionary.save(path)?;
                Ok(dictionary)
            }
            Err(primary_error) => {
                let Some(backup) = read_recovery_backup(path) else {
                    return Err(primary_error);
                };
                parse_dictionary(&backup)
                    .map(|(dictionary, _)| dictionary)
                    .map_err(|backup_error| {
                        AppError::from(format!(
                            "{primary_error}; backup recovery failed: {backup_error}"
                        ))
                    })
            }
        }
    }

    pub fn save(&self, path: &Path) -> AppResult<()> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AppError::from(format!("Create dictionary dir failed: {e}")))?;
        }
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::from(format!("Serialize dictionary failed: {e}")))?;
        let backup_path = dictionary_backup_path(path);
        let mut backed_up_primary = false;
        if path.is_file() {
            let previous = fs::read(path)
                .map_err(|e| AppError::from(format!("Read dictionary backup failed: {e}")))?;
            let previous_is_recoverable = std::str::from_utf8(&previous)
                .ok()
                .is_some_and(|text| parse_dictionary(text).is_ok());
            if previous_is_recoverable {
                atomic_replace(&backup_path, &previous)?;
                backed_up_primary = true;
            }
        }
        let backup_is_recoverable = fs::read_to_string(&backup_path)
            .ok()
            .is_some_and(|text| parse_dictionary(&text).is_ok());
        let needs_initial_backup = !backed_up_primary && !backup_is_recoverable;
        if needs_initial_backup {
            let prepared_backup = dictionary_prepared_backup_path(path);
            let committed_backup = dictionary_committed_backup_path(path);
            atomic_replace(&prepared_backup, data.as_bytes())?;
            if let Err(error) = atomic_replace(path, data.as_bytes()) {
                let _ = fs::remove_file(prepared_backup);
                return Err(error);
            }
            // This rename is the durable commit marker for the prepared recovery
            // copy. If it fails, roll back the first primary before reporting failure.
            if let Err(error) = fs::rename(&prepared_backup, &committed_backup) {
                let _ = fs::remove_file(path);
                return Err(AppError::from(format!(
                    "Commit dictionary backup failed: {error}"
                )));
            }
            // Canonical promotion is best-effort: the committed name remains an
            // authoritative recovery source if the destination is unavailable.
            let _ = fs::rename(committed_backup, backup_path);
            return Ok(());
        }
        atomic_replace(path, data.as_bytes())?;
        Ok(())
    }

    fn read_without_persisting(path: &Path) -> AppResult<Self> {
        let primary = fs::read_to_string(path)
            .map_err(|e| AppError::from(format!("Read dictionary failed: {e}")))?;
        match parse_dictionary(&primary) {
            Ok((dictionary, _)) => Ok(dictionary),
            Err(primary_error) => {
                let backup = read_recovery_backup(path).ok_or(primary_error)?;
                parse_dictionary(&backup).map(|(dictionary, _)| dictionary)
            }
        }
    }

    fn validate(&self) -> AppResult<()> {
        if self.schema_version != DICTIONARY_SCHEMA_VERSION {
            return Err(AppError::from(format!(
                "Unsupported dictionary schema version {}",
                self.schema_version
            )));
        }
        let mut entry_ids = HashSet::new();
        let mut canonical_keys = HashSet::new();
        for entry in &self.entries {
            validate_entry(entry, &mut entry_ids)?;
            let canonical = canonical_text(&entry.from);
            if !canonical_keys.insert(canonical.clone()) {
                return Err(AppError::from(format!(
                    "More than one dictionary entry uses canonical source key {canonical:?}"
                )));
            }
        }

        let mut conflict_ids = HashSet::new();
        for conflict in &self.migration_conflicts {
            if conflict.id.is_empty() || !conflict_ids.insert(conflict.id.as_str()) {
                return Err(AppError::from(
                    "Dictionary migration conflict IDs must be non-empty and unique",
                ));
            }
            if conflict.choices.len() < 2 {
                return Err(AppError::from(
                    "Dictionary migration conflicts require at least two choices",
                ));
            }
            if canonical_keys.contains(&conflict.canonical_from) {
                return Err(AppError::from(
                    "An unresolved dictionary migration conflict cannot also be active",
                ));
            }
            for choice in &conflict.choices {
                validate_entry(choice, &mut entry_ids)?;
                if canonical_text(&choice.from) != conflict.canonical_from {
                    return Err(AppError::from(
                        "Dictionary migration choice does not match its canonical source key",
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn list(&self) -> Vec<DictEntry> {
        let mut entries = self.entries.clone();
        entries.sort_by(|a, b| {
            a.from
                .to_ascii_lowercase()
                .cmp(&b.from.to_ascii_lowercase())
        });
        entries
    }

    pub fn resolve_migration_conflict(
        &mut self,
        resolution: &MigrationConflictResolution,
    ) -> AppResult<()> {
        let conflict_index = self
            .migration_conflicts
            .iter()
            .position(|conflict| conflict.id == resolution.conflict_id)
            .ok_or_else(|| AppError::from("Dictionary migration conflict no longer exists"))?;
        let selected = self.migration_conflicts[conflict_index]
            .choices
            .iter()
            .find(|entry| entry.id == resolution.selected_entry_id)
            .cloned()
            .ok_or_else(|| AppError::from("Selected dictionary mapping is not in this conflict"))?;
        if self
            .entries
            .iter()
            .any(|entry| canonical_text(&entry.from) == canonical_text(&selected.from))
        {
            return Err(AppError::from(
                "Dictionary key changed while resolving the migration conflict",
            ));
        }
        self.migration_conflicts.remove(conflict_index);
        self.entries.push(selected);
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    /// Upsert by case-insensitive `from` key.
    pub fn upsert(&mut self, from: &str, to: &str) -> AppResult<()> {
        let from = normalize_key(from)?;
        let to = to.trim();
        if to.is_empty() {
            return Err(AppError::from("Replacement text is empty"));
        }
        let canonical_from = canonical_text(&from);
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|entry| canonical_text(&entry.from) == canonical_from)
        {
            if canonical_text(&existing.to) == canonical_text(to) {
                return Ok(());
            }
            existing.from = from;
            existing.to = to.to_string();
            if existing.origin == EntryOrigin::Tuning {
                existing.edit_state = EntryEditState::ModifiedAfterVerification;
                existing.verified_fingerprints.clear();
            }
            existing.version = existing.version.saturating_add(1);
        } else {
            self.entries.push(DictEntry {
                id: Uuid::new_v4().to_string(),
                from,
                to: to.to_string(),
                origin: EntryOrigin::Manual,
                edit_state: EntryEditState::Unmodified,
                verified_fingerprints: Vec::new(),
                version: 1,
            });
        }
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    pub fn edit_entry(
        &mut self,
        identity: &DictionaryEntryIdentity,
        from: &str,
        to: &str,
    ) -> AppResult<()> {
        let from = normalize_key(from)?;
        let to = to.trim();
        if to.is_empty() {
            return Err(AppError::from("Replacement text is empty"));
        }
        let entry_index = self.entry_index_for_identity(identity)?;
        let entry = &self.entries[entry_index];
        let canonical_from = canonical_text(&from);
        if self.entries.iter().enumerate().any(|(index, entry)| {
            index != entry_index && canonical_text(&entry.from) == canonical_from
        }) {
            return Err(AppError::from(
                "Another dictionary entry already uses that source text",
            ));
        }
        if entry.from == from && entry.to == to {
            return Ok(());
        }

        let entry = &mut self.entries[entry_index];
        entry.from = from;
        entry.to = to.to_string();
        if entry.origin == EntryOrigin::Tuning {
            entry.edit_state = EntryEditState::ModifiedAfterVerification;
            entry.verified_fingerprints.clear();
        }
        entry.version = entry.version.saturating_add(1);
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    pub fn remove_entry(&mut self, identity: &DictionaryEntryIdentity) -> AppResult<()> {
        let entry_index = self.entry_index_for_identity(identity)?;
        self.entries.remove(entry_index);
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    fn entry_index_for_identity(&self, identity: &DictionaryEntryIdentity) -> AppResult<usize> {
        let entry_index = self
            .entries
            .iter()
            .position(|entry| entry.id == identity.id)
            .ok_or_else(|| AppError::from("Dictionary entry no longer exists"))?;
        let entry = &self.entries[entry_index];
        if entry.version != identity.version {
            return Err(AppError::from(format!(
                "Dictionary entry changed concurrently (expected version {}, found {})",
                identity.version, entry.version
            )));
        }
        Ok(entry_index)
    }

    /// Remove by case-insensitive `from` key. Returns true if something was removed.
    pub fn remove(&mut self, from: &str) -> bool {
        let needle = canonical_text(from);
        let before = self.entries.len();
        self.entries
            .retain(|entry| canonical_text(&entry.from) != needle);
        let removed = self.entries.len() != before;
        if removed {
            self.revision = self.revision.saturating_add(1);
        }
        removed
    }

    /// Apply entries active under the current Recognition Fingerprint.
    pub fn apply_for_fingerprint(
        &self,
        text: &str,
        fingerprint: Option<&RecognitionFingerprint>,
    ) -> String {
        let entries = self
            .entries
            .iter()
            .filter(|entry| entry.is_active_for(fingerprint))
            .map(|entry| (entry.from.as_str(), entry.to.as_str()))
            .collect::<Vec<_>>();
        apply_ordered_mappings(text, &entries)
    }

    /// Apply the ordinary active dictionary first, then an ephemeral Tuning
    /// overlay through the same matcher and ordering. Overlay keys shadow an
    /// ordinary entry with the same canonical source without mutating the
    /// Personal Dictionary used by ordinary dictation.
    pub fn apply_tuning_overlay(
        &self,
        text: &str,
        fingerprint: Option<&RecognitionFingerprint>,
        overlay: &[(&str, &str)],
    ) -> TuningOverlayApplication {
        let shadowed = overlay
            .iter()
            .map(|(from, _)| canonical_text(from))
            .collect::<HashSet<_>>();
        let base = self
            .entries
            .iter()
            .filter(|entry| {
                entry.is_active_for(fingerprint) && !shadowed.contains(&canonical_text(&entry.from))
            })
            .map(|entry| (entry.from.as_str(), entry.to.as_str()))
            .collect::<Vec<_>>();
        let pre_overlay = apply_ordered_mappings(text, &base);
        let post_overlay = apply_ordered_mappings(&pre_overlay, overlay);
        TuningOverlayApplication {
            pre_overlay,
            post_overlay,
        }
    }
}

fn apply_ordered_mappings(text: &str, mappings: &[(&str, &str)]) -> String {
    let mut ordered = mappings.to_vec();
    ordered.sort_by(|(left_from, _), (right_from, _)| {
        right_from
            .chars()
            .count()
            .cmp(&left_from.chars().count())
            .then_with(|| left_from.cmp(right_from))
    });
    ordered
        .into_iter()
        .fold(text.to_owned(), |out, (from, to)| {
            replace_phrase(&out, from, to)
        })
}

fn parse_dictionary(data: &str) -> AppResult<(Dictionary, bool)> {
    let value: serde_json::Value = serde_json::from_str(data)
        .map_err(|e| AppError::from(format!("Parse dictionary failed: {e}")))?;
    if value.get("schema_version").is_some() {
        let dictionary: Dictionary = serde_json::from_value(value)
            .map_err(|e| AppError::from(format!("Parse dictionary failed: {e}")))?;
        dictionary.validate()?;
        return Ok((dictionary, false));
    }

    let legacy: LegacyDictionary = serde_json::from_value(value)
        .map_err(|e| AppError::from(format!("Parse legacy dictionary failed: {e}")))?;
    Ok((migrate_legacy(legacy), true))
}

fn validate_entry<'a>(entry: &'a DictEntry, ids: &mut HashSet<&'a str>) -> AppResult<()> {
    if entry.id.is_empty() || !ids.insert(entry.id.as_str()) {
        return Err(AppError::from(
            "Dictionary entry IDs must be non-empty and unique",
        ));
    }
    if entry.version == 0 {
        return Err(AppError::from("Dictionary entry versions start at one"));
    }
    if canonical_text(&entry.from).is_empty() || entry.to.trim().is_empty() {
        return Err(AppError::from(
            "Dictionary source and replacement text must not be empty",
        ));
    }
    if (entry.origin == EntryOrigin::Manual
        || entry.edit_state == EntryEditState::ModifiedAfterVerification)
        && !entry.verified_fingerprints.is_empty()
    {
        return Err(AppError::from(
            "Manual or explicitly edited dictionary entries cannot retain Tuning verification",
        ));
    }
    let mut verified_fingerprints = HashSet::new();
    for verified in &entry.verified_fingerprints {
        let fingerprint = verified.fingerprint.as_str();
        if fingerprint.trim().is_empty() {
            return Err(AppError::from(
                "Verified Recognition Fingerprint identities must not be empty",
            ));
        }
        if !verified_fingerprints.insert(fingerprint) {
            return Err(AppError::from(
                "Verified Recognition Fingerprint identities must be unique per entry",
            ));
        }
    }
    Ok(())
}

impl DictEntry {
    pub(crate) fn is_active_for(&self, fingerprint: Option<&RecognitionFingerprint>) -> bool {
        if self.origin == EntryOrigin::Manual
            || self.edit_state == EntryEditState::ModifiedAfterVerification
        {
            return true;
        }
        fingerprint.is_some_and(|current| {
            self.verified_fingerprints
                .iter()
                .any(|verified| &verified.fingerprint == current)
        })
    }

    pub(crate) fn has_equivalent_mapping(&self, from: &str, to: &str) -> bool {
        canonical_text(&self.from) == canonical_text(from)
            && canonical_text(&self.to) == canonical_text(to)
    }
}

fn migrate_legacy(legacy: LegacyDictionary) -> Dictionary {
    struct LegacyGroup {
        canonical_from: String,
        choices: Vec<(String, DictEntry)>,
    }

    let mut groups: Vec<LegacyGroup> = Vec::new();
    for legacy_entry in legacy.entries {
        let canonical_from = canonical_text(&legacy_entry.from);
        let canonical_to = canonical_text(&legacy_entry.to);
        let group_index = groups
            .iter()
            .position(|group| group.canonical_from == canonical_from)
            .unwrap_or_else(|| {
                groups.push(LegacyGroup {
                    canonical_from: canonical_from.clone(),
                    choices: Vec::new(),
                });
                groups.len() - 1
            });
        let group = &mut groups[group_index];
        if group
            .choices
            .iter()
            .any(|(existing_to, _)| *existing_to == canonical_to)
        {
            continue;
        }
        group.choices.push((
            canonical_to,
            DictEntry {
                id: Uuid::new_v4().to_string(),
                from: legacy_entry.from,
                to: legacy_entry.to,
                origin: EntryOrigin::Manual,
                edit_state: EntryEditState::Unmodified,
                verified_fingerprints: Vec::new(),
                version: 1,
            },
        ));
    }

    let mut entries = Vec::new();
    let mut migration_conflicts = Vec::new();
    for mut group in groups {
        if group.choices.len() == 1 {
            entries.push(group.choices.remove(0).1);
        } else {
            migration_conflicts.push(MigrationConflict {
                id: Uuid::new_v4().to_string(),
                canonical_from: group.canonical_from,
                choices: group.choices.into_iter().map(|(_, entry)| entry).collect(),
            });
        }
    }

    Dictionary {
        schema_version: DICTIONARY_SCHEMA_VERSION,
        revision: 1,
        entries,
        migration_conflicts,
    }
}

pub(crate) fn canonical_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn dictionary_backup_path(path: &Path) -> PathBuf {
    let mut backup = path.as_os_str().to_os_string();
    backup.push(".bak");
    PathBuf::from(backup)
}

fn dictionary_prepared_backup_path(path: &Path) -> PathBuf {
    let mut prepared = path.as_os_str().to_os_string();
    prepared.push(".bak.prepared");
    PathBuf::from(prepared)
}

fn dictionary_committed_backup_path(path: &Path) -> PathBuf {
    let mut committed = path.as_os_str().to_os_string();
    committed.push(".bak.committed");
    PathBuf::from(committed)
}

fn read_recovery_backup(path: &Path) -> Option<String> {
    fs::read_to_string(dictionary_backup_path(path))
        .ok()
        .or_else(|| fs::read_to_string(dictionary_committed_backup_path(path)).ok())
}

fn reconcile_prepared_backup(path: &Path, primary_data: &str) {
    let prepared = dictionary_prepared_backup_path(path);
    let Ok(prepared_data) = fs::read_to_string(&prepared) else {
        return;
    };
    if prepared_data != primary_data {
        let _ = fs::remove_file(prepared);
        return;
    }
    let committed = dictionary_committed_backup_path(path);
    if fs::rename(&prepared, &committed).is_ok() {
        let _ = fs::rename(committed, dictionary_backup_path(path));
    }
}

fn atomic_replace(path: &Path, data: &[u8]) -> AppResult<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|e| AppError::from(format!("Create dictionary dir failed: {e}")))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("dictionary.json");
    let temporary = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let result = (|| -> AppResult<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| AppError::from(format!("Create dictionary temp file failed: {e}")))?;
        file.write_all(data)
            .map_err(|e| AppError::from(format!("Write dictionary temp file failed: {e}")))?;
        file.sync_all()
            .map_err(|e| AppError::from(format!("Sync dictionary temp file failed: {e}")))?;
        fs::rename(&temporary, path)
            .map_err(|e| AppError::from(format!("Replace dictionary failed: {e}")))?;
        if let Ok(directory) = fs::File::open(parent) {
            directory
                .sync_all()
                .map_err(|e| AppError::from(format!("Sync dictionary directory failed: {e}")))?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn normalize_key(from: &str) -> AppResult<String> {
    let from = from.split_whitespace().collect::<Vec<_>>().join(" ");
    if from.is_empty() {
        return Err(AppError::from("Dictionary key is empty"));
    }
    Ok(from)
}

/// Case-insensitive whole-phrase replace; preserves simple capitalization cues.
fn replace_phrase(text: &str, from: &str, to: &str) -> String {
    let from = from.trim();
    let to = to.trim();
    if from.is_empty() || to.is_empty() {
        return text.to_string();
    }

    let pattern = format!(r"(?i)\b{}\b", regex::escape(from));
    let Ok(re) = Regex::new(&pattern) else {
        return text.to_string();
    };

    re.replace_all(text, |caps: &regex::Captures| {
        let matched = caps.get(0).map(|m| m.as_str()).unwrap_or(from);
        adapt_casing(matched, to)
    })
    .into_owned()
}

/// If the STT match is ALL CAPS or Title Case, nudge the replacement similarly.
fn adapt_casing(matched: &str, replacement: &str) -> String {
    if matched
        .chars()
        .all(|c| !c.is_alphabetic() || c.is_uppercase())
        && matched.chars().any(|c| c.is_alphabetic())
    {
        return replacement.to_uppercase();
    }
    let mut chars = matched.chars();
    if let (Some(first), rest) = (chars.next(), chars.as_str()) {
        if first.is_uppercase() && rest.chars().all(|c| !c.is_alphabetic() || c.is_lowercase()) {
            // Title-ish single word: capitalize first letter of replacement only
            // when replacement is all-lowercase.
            if replacement
                .chars()
                .all(|c| !c.is_alphabetic() || c.is_lowercase())
            {
                let mut r = replacement.chars();
                if let Some(rf) = r.next() {
                    return format!("{}{}", rf.to_uppercase(), r.as_str());
                }
            }
        }
    }
    replacement.to_string()
}

/// Default path: `~/Library/Application Support/eaglescribe/dictionary.json` (macOS)
/// or platform equivalent via `dirs::data_local_dir`.
pub fn default_dictionary_path() -> PathBuf {
    if let Some(data) = dirs::data_local_dir() {
        return data.join("eaglescribe").join("dictionary.json");
    }
    PathBuf::from("dictionary.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_dictionary_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir()
            .join(format!("eaglescribe-dictionary-{label}-{nanos}"))
            .join("dictionary.json")
    }

    #[test]
    fn tuning_overlay_uses_production_order_without_activating_staged_rules() {
        let fingerprint = RecognitionFingerprint::from_stable_id("recognition-current");
        let dictionary = Dictionary::default();

        assert_eq!(
            dictionary.apply_for_fingerprint("a quick chip", Some(&fingerprint)),
            "a quick chip"
        );

        let applied = dictionary.apply_tuning_overlay(
            "a quick chip",
            Some(&fingerprint),
            &[("quick chip", "quick ship")],
        );
        assert_eq!(applied.pre_overlay, "a quick chip");
        assert_eq!(applied.post_overlay, "a quick ship");
        assert!(dictionary.entries.is_empty());
    }

    #[test]
    fn legacy_dictionary_migrates_without_rewriting_display_text() {
        let path = unique_dictionary_path("legacy-migration");
        fs::create_dir_all(path.parent().unwrap()).expect("create test directory");
        let legacy = r#"{
  "entries": [
    { "from": "  Eagle   Scribe  ", "to": "EagleScribe  " }
  ]
}"#;
        fs::write(&path, legacy).expect("write legacy dictionary");

        let dictionary = Dictionary::load(&path).expect("migrate legacy dictionary");

        assert_eq!(dictionary.schema_version, DICTIONARY_SCHEMA_VERSION);
        assert_eq!(dictionary.revision, 1);
        assert_eq!(dictionary.entries.len(), 1);
        let entry = &dictionary.entries[0];
        assert!(!entry.id.is_empty());
        assert_eq!(entry.from, "  Eagle   Scribe  ");
        assert_eq!(entry.to, "EagleScribe  ");
        assert_eq!(entry.origin, EntryOrigin::Manual);
        assert_eq!(entry.edit_state, EntryEditState::Unmodified);
        assert!(entry.verified_fingerprints.is_empty());
        assert_eq!(entry.version, 1);
        assert_eq!(
            fs::read_to_string(dictionary_backup_path(&path)).expect("read backup"),
            legacy
        );
        let persisted: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("read migrated dictionary"))
                .expect("parse migrated dictionary");
        assert_eq!(
            persisted["schema_version"].as_u64(),
            Some(u64::from(DICTIONARY_SCHEMA_VERSION))
        );

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn migration_coalesces_equivalents_and_requires_conflict_resolution() {
        let path = unique_dictionary_path("legacy-conflicts");
        fs::create_dir_all(path.parent().unwrap()).expect("create test directory");
        fs::write(
            &path,
            r#"{
  "entries": [
    { "from": "  CAFÉ   Noir ", "to": " Brand   Name " },
    { "from": "café noir", "to": "brand name" },
    { "from": "Project X", "to": "Project Ex" },
    { "from": "project   x", "to": "Project Ten" }
  ]
}"#,
        )
        .expect("write legacy dictionary");

        let mut dictionary = Dictionary::load(&path).expect("migrate legacy dictionary");

        assert_eq!(dictionary.entries.len(), 1);
        assert_eq!(dictionary.entries[0].from, "  CAFÉ   Noir ");
        assert_eq!(dictionary.entries[0].to, " Brand   Name ");
        assert_eq!(dictionary.migration_conflicts.len(), 1);
        let conflict = &dictionary.migration_conflicts[0];
        assert_eq!(conflict.canonical_from, "project x");
        assert_eq!(conflict.choices.len(), 2);
        assert!(dictionary
            .entries
            .iter()
            .all(|entry| canonical_text(&entry.from) != "project x"));

        let conflict_id = conflict.id.clone();
        let selected_id = conflict.choices[1].id.clone();
        dictionary
            .resolve_migration_conflict(&MigrationConflictResolution {
                conflict_id,
                selected_entry_id: selected_id.clone(),
            })
            .expect("resolve conflict explicitly");
        dictionary.save(&path).expect("save resolution");

        let reloaded = Dictionary::load(&path).expect("reload resolved dictionary");
        assert!(reloaded.migration_conflicts.is_empty());
        assert_eq!(reloaded.entries.len(), 2);
        let selected = reloaded
            .entries
            .iter()
            .find(|entry| entry.id == selected_id)
            .expect("selected mapping remains stable");
        assert_eq!(selected.from, "project   x");
        assert_eq!(selected.to, "Project Ten");
        assert_eq!(reloaded.revision, 2);

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn manual_lifecycle_preserves_identity_and_rejects_stale_changes() {
        let mut dictionary = Dictionary::default();
        dictionary.upsert("Café   Noir", "Brand").expect("add");
        let id = dictionary.entries[0].id.clone();

        dictionary
            .upsert("café noir", "New Brand")
            .expect("edit by canonical key");

        assert_eq!(dictionary.entries.len(), 1);
        assert_eq!(dictionary.entries[0].id, id);
        assert_eq!(dictionary.entries[0].version, 2);
        assert_eq!(dictionary.revision, 2);
        assert!(dictionary
            .edit_entry(
                &DictionaryEntryIdentity {
                    id: id.clone(),
                    version: 1,
                },
                "café noir",
                "Stale Brand",
            )
            .is_err());
        assert_eq!(dictionary.entries[0].to, "New Brand");

        dictionary.upsert("co-op", "coop").expect("add hyphen key");
        dictionary
            .upsert("co op", "cooperative")
            .expect("add phrase key");
        assert_eq!(dictionary.entries.len(), 3);
        let phrase_id = dictionary
            .entries
            .iter()
            .find(|entry| entry.from == "co op")
            .unwrap()
            .id
            .clone();
        assert!(dictionary
            .edit_entry(
                &DictionaryEntryIdentity {
                    id: phrase_id,
                    version: 1,
                },
                "CAFÉ NOIR",
                "collision",
            )
            .is_err());

        dictionary
            .remove_entry(&DictionaryEntryIdentity {
                id: id.clone(),
                version: 2,
            })
            .expect("remove current entry");
        assert!(dictionary.entries.iter().all(|entry| entry.id != id));
        assert!(dictionary
            .remove_entry(&DictionaryEntryIdentity { id, version: 2 })
            .is_err());
    }

    #[test]
    fn tuning_rule_scope_clears_after_explicit_edit() {
        let mut dictionary = Dictionary::default();
        let model_a = RecognitionFingerprint::from_stable_id("model-a");
        let model_b = RecognitionFingerprint::from_stable_id("model-b");
        dictionary.entries.push(DictEntry {
            id: "tuning-rule-1".into(),
            from: "eagle scribe".into(),
            to: "EagleScribe".into(),
            origin: EntryOrigin::Tuning,
            edit_state: EntryEditState::Unmodified,
            verified_fingerprints: vec![VerifiedRecognitionFingerprint {
                fingerprint: model_a.clone(),
                verified_at_ms: 123,
            }],
            version: 1,
        });

        assert_eq!(
            dictionary.apply_for_fingerprint("use eagle scribe", Some(&model_a)),
            "use EagleScribe"
        );
        assert_eq!(
            dictionary.apply_for_fingerprint("use eagle scribe", Some(&model_b)),
            "use eagle scribe"
        );

        dictionary
            .edit_entry(
                &DictionaryEntryIdentity {
                    id: "tuning-rule-1".into(),
                    version: 1,
                },
                "eagle scribe",
                "Eagle Scribe",
            )
            .expect("explicit user edit");
        let edited = &dictionary.entries[0];
        assert_eq!(edited.id, "tuning-rule-1");
        assert_eq!(edited.origin, EntryOrigin::Tuning);
        assert_eq!(edited.edit_state, EntryEditState::ModifiedAfterVerification);
        assert!(edited.verified_fingerprints.is_empty());
        assert_eq!(edited.version, 2);
        assert_eq!(
            dictionary.apply_for_fingerprint("use eagle scribe", None),
            "use Eagle Scribe"
        );
    }

    #[test]
    fn corrupt_primary_recovers_the_previous_atomic_write() {
        let path = unique_dictionary_path("backup-recovery");
        let mut dictionary = Dictionary::default();
        dictionary.upsert("eagle scribe", "EagleScribe").unwrap();
        let stable_id = dictionary.entries[0].id.clone();
        dictionary.save(&path).expect("save first version");
        dictionary
            .upsert("eagle scribe", "Eagle Scribe")
            .expect("edit");
        dictionary.save(&path).expect("save second version");
        fs::write(&path, "{ interrupted").expect("corrupt primary");

        let mut recovered = Dictionary::load(&path).expect("recover backup");

        assert_eq!(recovered.revision, 1);
        assert_eq!(recovered.entries[0].id, stable_id);
        assert_eq!(recovered.entries[0].to, "EagleScribe");
        assert!(dictionary_backup_path(&path).is_file());

        recovered
            .upsert("eagle scribe", "Recovered update")
            .expect("edit recovered dictionary");
        recovered.save(&path).expect("save recovered dictionary");
        fs::write(&path, "{ interrupted again").expect("corrupt primary again");
        let recovered_again = Dictionary::load(&path).expect("backup remains recoverable");
        assert_eq!(recovered_again.entries[0].id, stable_id);
        assert_eq!(recovered_again.entries[0].to, "EagleScribe");

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn first_dictionary_write_has_a_recoverable_backup() {
        let path = unique_dictionary_path("first-write-backup");
        let mut dictionary = Dictionary::default();
        dictionary.upsert("local first", "local-first").unwrap();

        dictionary.save(&path).expect("save new dictionary");
        fs::write(&path, "{ interrupted").expect("corrupt primary");

        let recovered = Dictionary::load(&path).expect("recover first write");
        assert_eq!(recovered.entries[0].to, "local-first");

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn failed_first_write_does_not_recover_uncommitted_mapping() {
        let path = unique_dictionary_path("failed-first-write");
        fs::create_dir_all(&path).expect("block primary file creation");
        let mut dictionary = Dictionary::default();
        dictionary.upsert("should not", "appear").unwrap();

        assert!(dictionary.save(&path).is_err());
        let recovered = Dictionary::load(&path).expect("load after failed write");

        assert!(recovered.entries.is_empty());

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn first_write_remains_committed_when_backup_promotion_fails() {
        let path = unique_dictionary_path("backup-promotion-failure");
        fs::create_dir_all(path.parent().unwrap()).expect("create test directory");
        fs::create_dir(dictionary_backup_path(&path)).expect("block backup promotion");
        let mut dictionary = Dictionary::default();
        dictionary.upsert("committed", "mapping").unwrap();

        dictionary.save(&path).expect("primary commit succeeds");
        fs::remove_file(&path).expect("lose primary after commit");
        let recovered = Dictionary::load(&path).expect("recover provisional backup");

        assert_eq!(
            recovered.entries[0].to, "mapping",
            "successful write must remain committed and recoverable"
        );

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn startup_reconciles_prepared_backup_after_primary_commit_crash() {
        let path = unique_dictionary_path("prepared-backup-reconciliation");
        fs::create_dir_all(path.parent().unwrap()).expect("create test directory");
        let mut dictionary = Dictionary::default();
        dictionary.upsert("survives", "restart").unwrap();
        let data = serde_json::to_string_pretty(&dictionary).unwrap();
        fs::write(&path, &data).expect("simulate committed primary");
        fs::write(dictionary_prepared_backup_path(&path), &data)
            .expect("simulate pre-marker crash");

        Dictionary::load(&path).expect("reconcile prepared backup");
        fs::remove_file(&path).expect("lose primary after reconciliation");
        let recovered = Dictionary::load(&path).expect("recover reconciled backup");

        assert_eq!(recovered.entries[0].to, "restart");

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn runtime_load_keeps_legacy_mappings_when_migration_cannot_persist() {
        let path = unique_dictionary_path("migration-write-failure");
        fs::create_dir_all(path.parent().unwrap()).expect("create test directory");
        fs::write(
            &path,
            r#"{"entries":[{"from":"eagle scribe","to":"EagleScribe"}]}"#,
        )
        .expect("write legacy dictionary");
        fs::create_dir(dictionary_backup_path(&path)).expect("block backup file creation");

        let (dictionary, warning) = Dictionary::load_for_runtime(&path);

        assert_eq!(
            dictionary.apply_for_fingerprint("use eagle scribe", None),
            "use EagleScribe"
        );
        assert!(warning
            .as_deref()
            .is_some_and(|message| message.contains("Replace dictionary failed")));

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn versioned_dictionary_rejects_duplicate_canonical_keys() {
        let path = unique_dictionary_path("duplicate-versioned-keys");
        fs::create_dir_all(path.parent().unwrap()).expect("create test directory");
        fs::write(
            &path,
            r#"{
  "schema_version": 1,
  "revision": 7,
  "entries": [
    {
      "id": "one", "from": "Café Noir", "to": "A", "origin": "manual",
      "edit_state": "unmodified", "verified_fingerprints": [], "version": 1
    },
    {
      "id": "two", "from": " café   noir ", "to": "B", "origin": "manual",
      "edit_state": "unmodified", "verified_fingerprints": [], "version": 1
    }
  ],
  "migration_conflicts": []
}"#,
        )
        .expect("write invalid versioned dictionary");

        let error = Dictionary::load(&path).expect_err("duplicate key must fail");
        assert!(error.to_string().contains("canonical source key"));

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn replaces_phrase_case_insensitive() {
        let mut d = Dictionary::default();
        d.upsert("eagle scribe", "EagleScribe").unwrap();
        assert_eq!(
            d.apply_for_fingerprint("I love eagle scribe so much", None),
            "I love EagleScribe so much"
        );
        assert_eq!(
            d.apply_for_fingerprint("EAGLE SCRIBE rocks", None),
            "EAGLESCRIBE rocks"
        );
    }

    #[test]
    fn longer_phrase_wins() {
        let mut d = Dictionary::default();
        d.upsert("type", "TYPE").unwrap();
        d.upsert("eagle scribe", "EagleScribe").unwrap();
        assert_eq!(
            d.apply_for_fingerprint("use eagle scribe please", None),
            "use EagleScribe please"
        );
    }

    #[test]
    fn upsert_and_remove() {
        let mut d = Dictionary::default();
        d.upsert("foo", "Foo").unwrap();
        d.upsert("FOO", "FOO!").unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].to, "FOO!");
        assert!(d.remove("foo"));
        assert!(!d.remove("foo"));
        assert!(d.entries.is_empty());
    }

    #[test]
    fn word_boundaries() {
        let mut d = Dictionary::default();
        d.upsert("cat", "feline").unwrap();
        assert_eq!(
            d.apply_for_fingerprint("the cat scattered", None),
            "the feline scattered"
        );
    }
}
