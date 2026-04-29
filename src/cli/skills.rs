use std::{
    collections::BTreeMap,
    env, fs,
    io::{Seek, Write},
    path::{Component, Path, PathBuf},
    process,
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    compiler::resolve_compile_assets,
    contracts::{
        LearningRequestAction, LearningRequestDocument, Plane, Timestamp, validate_safe_identifier,
    },
    workspace::{QueueStore, WorkspacePaths, atomic_write_text},
};

use super::parser::ParsedArgs;

const REMOTE_SKILLS_INDEX_FILENAME: &str = "remote_skills_index.md";
const REMOTE_SKILLS_INDEX_ENV: &str = "MILLRACE_REMOTE_SKILLS_INDEX_PATH";
const REMOTE_SKILLS_ROOT_ENV: &str = "MILLRACE_REMOTE_SKILLS_ROOT";
const SOURCE_SKILLS_DIR_ENV: &str = "MILLRACE_SOURCE_SKILLS_DIR";

pub fn skills_command_lines(
    command: &str,
    parsed: &ParsedArgs,
    paths: &WorkspacePaths,
) -> Result<Vec<String>, String> {
    match command {
        "ls" => skills_ls(parsed, paths),
        "show" => skills_show(parsed, paths),
        "search" => skills_search(parsed, paths),
        "install" => skills_install(parsed, paths),
        "refresh-remote-index" => skills_refresh_remote_index(paths),
        "create" => skills_create(parsed, paths),
        "improve" => skills_improve(parsed, paths),
        "promote" => skills_promote(parsed, paths),
        "export" => skills_export(parsed, paths),
        _ => Err(format!("unknown skills command `{command}`")),
    }
}

fn skills_ls(parsed: &ParsedArgs, paths: &WorkspacePaths) -> Result<Vec<String>, String> {
    let skills_dir = selected_skills_dir(paths, parsed.value("--target"))?;
    Ok(skill_locations(&skills_dir)?
        .into_iter()
        .filter(|(_, relative_path)| skills_dir.join(relative_path).is_file())
        .map(|(skill_id, _)| skill_id)
        .collect())
}

fn skills_show(parsed: &ParsedArgs, paths: &WorkspacePaths) -> Result<Vec<String>, String> {
    let skill_id = required_positional(parsed)?;
    validate_skill_id(skill_id)?;
    let skills_dir = selected_skills_dir(paths, parsed.value("--target"))?;
    let skill_path = find_skill_path(&skills_dir, skill_id)?
        .ok_or_else(|| format!("skill not found: {skill_id}"))?;
    let mut lines = vec![
        format!("skill_id: {skill_id}"),
        format!("path: {}", skill_path.display()),
    ];
    if let Some(title) = first_markdown_heading(&skill_path)? {
        lines.push(format!("title: {title}"));
    }
    Ok(lines)
}

fn skills_search(parsed: &ParsedArgs, paths: &WorkspacePaths) -> Result<Vec<String>, String> {
    let query = required_positional(parsed)?.trim().to_lowercase();
    if query.is_empty() {
        return Err("search query is required".to_owned());
    }
    let skills_dir = selected_skills_dir(paths, parsed.value("--target"))?;
    let mut matches = Vec::new();
    for (skill_id, relative_path) in skill_locations(&skills_dir)? {
        let skill_path = skills_dir.join(relative_path);
        if !skill_path.is_file() {
            continue;
        }
        let haystack = fs::read_to_string(&skill_path)
            .map_err(|error| format!("failed to read {}: {error}", skill_path.display()))?
            .to_lowercase();
        if skill_id.to_lowercase().contains(&query) || haystack.contains(&query) {
            matches.push(skill_id);
        }
    }
    Ok(matches)
}

fn skills_install(parsed: &ParsedArgs, paths: &WorkspacePaths) -> Result<Vec<String>, String> {
    let skill_ref = required_positional(parsed)?;
    let destination_root = selected_skills_dir(paths, parsed.value("--target"))?;
    let force = parsed.value("--force").is_some();
    let update = parsed.value("--update").is_some();

    match resolve_local_skill_source(skill_ref) {
        LocalSkillSource::Found(source) => {
            let skill_id = skill_id_for_source(&source)?;
            validate_skill_id(&skill_id)?;
            let destination = destination_root.join(&skill_id);
            install_local_skill(&source, &destination, force || update)?;
            sync_skills_index(
                &destination_root,
                &skill_id,
                &format!("{skill_id}/SKILL.md"),
            )?;
            append_skill_operation(
                &destination_root,
                "install",
                &skill_id,
                &source.display().to_string(),
                &destination.display().to_string(),
                BTreeMap::new(),
            )?;
            Ok(vec![
                format!("installed_skill: {skill_id}"),
                format!("path: {}", destination.display()),
            ])
        }
        LocalSkillSource::Malformed(message) => Err(message),
        LocalSkillSource::Missing => {
            let result = install_remote_skill(paths, &destination_root, skill_ref, force, update)?;
            Ok(vec![
                format!("installed_skill: {}", result.skill_id),
                "source: remote".to_owned(),
                format!("source_index: {}", result.source_index),
                format!("path: {}", result.destination.display()),
            ])
        }
    }
}

fn skills_refresh_remote_index(paths: &WorkspacePaths) -> Result<Vec<String>, String> {
    let destination = paths.skills_dir.join(REMOTE_SKILLS_INDEX_FILENAME);
    let (index_text, source) = if let Some(source) = env::var_os(REMOTE_SKILLS_INDEX_ENV) {
        let source = PathBuf::from(source);
        let index_text = fs::read_to_string(&source)
            .map_err(|error| format!("failed to read remote skills index fixture: {error}"))?;
        (index_text, source.display().to_string())
    } else if destination.is_file() {
        let index_text = fs::read_to_string(&destination)
            .map_err(|error| format!("failed to read cached remote skills index: {error}"))?;
        (index_text, destination.display().to_string())
    } else {
        return Err(format!(
            "remote skill index source is unavailable; set {REMOTE_SKILLS_INDEX_ENV}"
        ));
    };

    atomic_write_text(&destination, &index_text).map_err(|error| error.to_string())?;
    let mut extra = BTreeMap::new();
    extra.insert(
        "entry_count".to_owned(),
        json!(parse_remote_skill_index(&index_text).len()),
    );
    append_skill_operation(
        &paths.skills_dir,
        "refresh_remote_index",
        "remote_skills_index",
        &source,
        &destination.display().to_string(),
        extra,
    )?;
    Ok(vec![format!(
        "remote_skills_index: {}",
        destination.display()
    )])
}

fn skills_create(parsed: &ParsedArgs, paths: &WorkspacePaths) -> Result<Vec<String>, String> {
    let prompt = required_positional(parsed)?;
    require_learning_mode(paths, parsed.value("--mode"))?;
    let document =
        learning_request_document(LearningRequestAction::Create, "Create skill", prompt, None)?;
    enqueue_learning_request(paths, &document)
}

fn skills_improve(parsed: &ParsedArgs, paths: &WorkspacePaths) -> Result<Vec<String>, String> {
    let skill_id = required_positional(parsed)?;
    validate_skill_id(skill_id)?;
    require_learning_mode(paths, parsed.value("--mode"))?;
    let document = learning_request_document(
        LearningRequestAction::Improve,
        &format!("Improve {skill_id}"),
        &format!("Improve installed skill {skill_id}."),
        Some(skill_id.to_owned()),
    )?;
    enqueue_learning_request(paths, &document)
}

fn skills_promote(parsed: &ParsedArgs, paths: &WorkspacePaths) -> Result<Vec<String>, String> {
    let skill_id = required_positional(parsed)?;
    validate_skill_id(skill_id)?;
    let source = paths.skills_dir.join(skill_id);
    if !source.join("SKILL.md").is_file() {
        return Err(format!("workspace skill not found: {skill_id}"));
    }
    let destination_root = source_skills_dir()?;
    let destination = destination_root.join(skill_id);
    if destination.exists() {
        return Err(format!("source skill already exists: {skill_id}"));
    }
    copy_dir_all(&source, &destination)?;
    sync_skills_index(&destination_root, skill_id, &format!("{skill_id}/SKILL.md"))?;
    let promoted_files = relative_package_files(&destination)?;
    let mut file_hashes = BTreeMap::new();
    for relative_path in &promoted_files {
        let path = destination.join(relative_path);
        let bytes = fs::read(&path)
            .map_err(|error| format!("failed to read promoted file {}: {error}", path.display()))?;
        file_hashes.insert(relative_path.clone(), hex_string(Sha256::digest(&bytes)));
    }
    let mut extra = BTreeMap::new();
    extra.insert("operator_controlled".to_owned(), json!(true));
    extra.insert("promotion_source".to_owned(), json!("workspace"));
    extra.insert("promotion_destination".to_owned(), json!("source"));
    extra.insert("promoted_files".to_owned(), json!(promoted_files));
    extra.insert("file_sha256".to_owned(), json!(file_hashes));
    append_skill_operation(
        &destination_root,
        "promote",
        skill_id,
        &source.display().to_string(),
        &destination.display().to_string(),
        extra,
    )?;
    Ok(vec![
        format!("promoted_skill: {skill_id}"),
        format!("path: {}", destination.display()),
    ])
}

fn skills_export(parsed: &ParsedArgs, paths: &WorkspacePaths) -> Result<Vec<String>, String> {
    let skill_id = required_positional(parsed)?;
    validate_skill_id(skill_id)?;
    let source = paths.skills_dir.join(skill_id);
    if !source.join("SKILL.md").is_file() {
        return Err(format!("skill not found: {skill_id}"));
    }
    let archive = export_archive_path(paths, skill_id, parsed.value("--output"));
    write_zip_archive(&source, &archive)?;
    append_skill_operation(
        &paths.skills_dir,
        "export",
        skill_id,
        &source.display().to_string(),
        &archive.display().to_string(),
        BTreeMap::new(),
    )?;
    Ok(vec![format!("exported_skill: {}", archive.display())])
}

fn selected_skills_dir(paths: &WorkspacePaths, target: Option<&str>) -> Result<PathBuf, String> {
    match target
        .unwrap_or("workspace")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "workspace" => Ok(paths.skills_dir.clone()),
        "source" => source_skills_dir(),
        _ => Err("target must be workspace or source".to_owned()),
    }
}

fn source_skills_dir() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os(SOURCE_SKILLS_DIR_ENV) {
        let path = PathBuf::from(path);
        if path.is_dir() {
            return Ok(path);
        }
        return Err(format!(
            "cannot locate source skill asset directory: {}",
            path.display()
        ));
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/assets/baseline/skills");
    if path.is_dir() {
        Ok(path)
    } else {
        Err("cannot locate source skill asset directory".to_owned())
    }
}

fn required_positional(parsed: &ParsedArgs) -> Result<&str, String> {
    parsed
        .positionals
        .first()
        .map(String::as_str)
        .ok_or_else(|| "missing required argument `SKILL_ARG`".to_owned())
}

fn validate_skill_id(skill_id: &str) -> Result<(), String> {
    if skill_id == "." || skill_id == ".." || skill_id.contains('/') || skill_id.contains('\\') {
        return Err(format!("unsafe skill id: {skill_id}"));
    }
    validate_safe_identifier(skill_id, "skill_id")
        .map(|_| ())
        .map_err(|error| format!("unsafe skill id: {skill_id}: {error}"))
}

#[derive(Debug)]
enum LocalSkillSource {
    Found(PathBuf),
    Malformed(String),
    Missing,
}

fn resolve_local_skill_source(skill_ref: &str) -> LocalSkillSource {
    let candidate = PathBuf::from(skill_ref);
    if !candidate.exists() {
        return LocalSkillSource::Missing;
    }
    if candidate.is_dir() {
        if candidate.join("SKILL.md").is_file() {
            return LocalSkillSource::Found(candidate);
        }
        return LocalSkillSource::Malformed(format!(
            "malformed skill package: {} is missing SKILL.md",
            candidate.display()
        ));
    }
    if candidate.is_file() && candidate.file_name().is_some_and(|name| name == "SKILL.md") {
        if let Some(parent) = candidate.parent() {
            return LocalSkillSource::Found(parent.to_path_buf());
        }
    }
    LocalSkillSource::Malformed(format!(
        "malformed skill package: {} must be a skill directory or SKILL.md file",
        candidate.display()
    ))
}

fn skill_id_for_source(source: &Path) -> Result<String, String> {
    source
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("cannot derive skill id from {}", source.display()))
}

fn install_local_skill(source: &Path, destination: &Path, replace: bool) -> Result<(), String> {
    if destination.exists() && !replace {
        let skill_id = destination
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<unknown>");
        return Err(format!("skill already exists: {skill_id}"));
    }
    if same_existing_path(source, destination) {
        return Err("skill source and destination must differ".to_owned());
    }
    if destination.exists() {
        fs::remove_dir_all(destination)
            .map_err(|error| format!("failed to replace {}: {error}", destination.display()))?;
    }
    copy_dir_all(source, destination)
}

fn same_existing_path(left: &Path, right: &Path) -> bool {
    let Ok(left) = left.canonicalize() else {
        return false;
    };
    let Ok(right) = right.canonicalize() else {
        return false;
    };
    left == right
}

fn copy_dir_all(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.join("SKILL.md").is_file() {
        return Err(format!(
            "malformed skill package: {} is missing SKILL.md",
            source.display()
        ));
    }
    fs::create_dir_all(destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;
    for entry in sorted_read_dir(source)? {
        let target = destination.join(entry.file_name());
        let path = entry.path();
        if path.is_dir() {
            copy_dir_tree(&path, &target)?;
        } else if path.is_file() {
            fs::copy(&path, &target).map_err(|error| {
                format!(
                    "failed to copy {} to {}: {error}",
                    path.display(),
                    target.display()
                )
            })?;
        }
    }
    Ok(())
}

fn copy_dir_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;
    for entry in sorted_read_dir(source)? {
        let target = destination.join(entry.file_name());
        let path = entry.path();
        if path.is_dir() {
            copy_dir_tree(&path, &target)?;
        } else if path.is_file() {
            fs::copy(&path, &target).map_err(|error| {
                format!(
                    "failed to copy {} to {}: {error}",
                    path.display(),
                    target.display()
                )
            })?;
        }
    }
    Ok(())
}

fn sorted_read_dir(directory: &Path) -> Result<Vec<fs::DirEntry>, String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?;
    entries.sort_by_key(|entry| entry.path());
    Ok(entries)
}

fn skill_locations(skills_dir: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let mut locations = BTreeMap::new();
    for (skill_id, relative_path) in indexed_skill_locations(skills_dir)? {
        locations.entry(skill_id).or_insert(relative_path);
    }
    for skill_path in recursive_skill_files(skills_dir)? {
        let Some(skill_id) = skill_path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .map(ToOwned::to_owned)
        else {
            continue;
        };
        if validate_skill_id(&skill_id).is_err() {
            continue;
        }
        let relative = skill_path
            .strip_prefix(skills_dir)
            .map_err(|error| error.to_string())?
            .to_path_buf();
        locations.entry(skill_id).or_insert(relative);
    }
    Ok(locations)
}

fn indexed_skill_locations(skills_dir: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let index_path = skills_dir.join("skills_index.md");
    if !index_path.is_file() {
        return Ok(BTreeMap::new());
    }
    let text = fs::read_to_string(&index_path)
        .map_err(|error| format!("failed to read {}: {error}", index_path.display()))?;
    let mut locations = BTreeMap::new();
    for line in text.lines() {
        if let Some((skill_id, path)) = parse_index_bullet(line)? {
            locations.insert(skill_id, path);
            continue;
        }
        if let Some((skill_id, path)) = parse_index_table_row(line)? {
            locations.insert(skill_id, path);
        }
    }
    Ok(locations)
}

fn parse_index_bullet(line: &str) -> Result<Option<(String, PathBuf)>, String> {
    let stripped = line.trim();
    let Some(rest) = stripped.strip_prefix("- ") else {
        return Ok(None);
    };
    let Some((skill_id, skill_path)) = rest.split_once(':') else {
        return Ok(None);
    };
    let skill_id = strip_markdown_code(skill_id).trim().to_owned();
    if validate_skill_id(&skill_id).is_err() {
        return Ok(None);
    }
    Ok(Some((
        skill_id,
        normalize_index_skill_path(skill_path.trim())?,
    )))
}

fn parse_index_table_row(line: &str) -> Result<Option<(String, PathBuf)>, String> {
    let stripped = line.trim();
    if !stripped.starts_with('|') || !stripped.contains('`') {
        return Ok(None);
    }
    let columns = split_markdown_table_row(stripped);
    if columns.len() < 5
        || columns[0].eq_ignore_ascii_case("skill")
        || columns[0].chars().all(|ch| ch == '-' || ch == ' ')
    {
        return Ok(None);
    }
    let skill_id = strip_markdown_code(&columns[0]).trim().to_owned();
    if validate_skill_id(&skill_id).is_err() {
        return Ok(None);
    }
    Ok(Some((
        skill_id,
        normalize_index_skill_path(strip_markdown_code(&columns[3]).trim())?,
    )))
}

fn normalize_index_skill_path(raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    let relative = raw.strip_prefix("skills/").unwrap_or(raw);
    let path = Path::new(relative);
    reject_unsafe_relative_path(path, "skill index path")?;
    Ok(path.to_path_buf())
}

fn recursive_skill_files(skills_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    if !skills_dir.is_dir() {
        return Ok(files);
    }
    collect_skill_files(skills_dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_skill_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in sorted_read_dir(directory)? {
        let path = entry.path();
        if path.is_dir() {
            collect_skill_files(&path, files)?;
        } else if path.is_file() && path.file_name().is_some_and(|name| name == "SKILL.md") {
            files.push(path);
        }
    }
    Ok(())
}

fn find_skill_path(skills_dir: &Path, skill_id: &str) -> Result<Option<PathBuf>, String> {
    let locations = skill_locations(skills_dir)?;
    Ok(locations
        .get(skill_id)
        .map(|relative_path| skills_dir.join(relative_path))
        .filter(|path| path.is_file()))
}

fn first_markdown_heading(path: &Path) -> Result<Option<String>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    Ok(text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("# ")
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(ToOwned::to_owned)
    }))
}

fn sync_skills_index(skills_dir: &Path, skill_id: &str, skill_path: &str) -> Result<(), String> {
    fs::create_dir_all(skills_dir)
        .map_err(|error| format!("failed to create {}: {error}", skills_dir.display()))?;
    let index_path = skills_dir.join("skills_index.md");
    let existing =
        fs::read_to_string(&index_path).unwrap_or_else(|_| "# Skills Index\n".to_owned());
    let entry = format!("- {skill_id}: {skill_path}");
    if existing.lines().any(|line| line.trim() == entry) {
        return Ok(());
    }
    let payload = format!("{}\n{entry}\n", existing.trim_end());
    atomic_write_text(&index_path, &payload).map_err(|error| error.to_string())
}

fn append_skill_operation(
    skills_dir: &Path,
    operation: &str,
    skill_id: &str,
    source: &str,
    destination: &str,
    extra: BTreeMap<String, Value>,
) -> Result<(), String> {
    fs::create_dir_all(skills_dir)
        .map_err(|error| format!("failed to create {}: {error}", skills_dir.display()))?;
    let log_path = skills_dir.join("skill_operations.jsonl");
    let mut payload = BTreeMap::new();
    payload.insert("at".to_owned(), json!(timestamp_now()?));
    payload.insert("operation".to_owned(), json!(operation));
    payload.insert("skill_id".to_owned(), json!(skill_id));
    payload.insert("source".to_owned(), json!(source));
    payload.insert("destination".to_owned(), json!(destination));
    payload.extend(extra);
    let encoded = serde_json::to_string(&payload).map_err(|error| error.to_string())?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| format!("failed to open {}: {error}", log_path.display()))?;
    writeln!(file, "{encoded}")
        .map_err(|error| format!("failed to write {}: {error}", log_path.display()))
}

#[derive(Debug)]
struct RemoteInstallResult {
    skill_id: String,
    destination: PathBuf,
    source_index: String,
}

fn install_remote_skill(
    paths: &WorkspacePaths,
    destination_root: &Path,
    skill_ref: &str,
    force: bool,
    update: bool,
) -> Result<RemoteInstallResult, String> {
    let (index_text, index_source_path, source_index) = load_remote_index(paths)?;
    let entries = parse_remote_skill_index(&index_text);
    let entry = entries
        .into_iter()
        .find(|entry| entry.skill_id == skill_ref)
        .ok_or_else(|| format!("remote skill not found: {}", skill_ref.trim()))?;
    if entry.status != "available" {
        return Err(format!(
            "remote skill is not available: {} status={}",
            entry.skill_id, entry.status
        ));
    }
    validate_skill_id(&entry.skill_id)?;
    let source_dir = source_directory_for_remote_entry(&entry)?;
    let mirror_root = remote_mirror_root(index_source_path.as_deref())?;
    let source = mirror_root.join(&source_dir);
    if !source.join("SKILL.md").is_file() {
        return Err(format!(
            "remote skill package mirror not found: {}",
            source.display()
        ));
    }

    let destination = destination_root.join(&entry.skill_id);
    if destination.exists() && !(force || update) {
        return Err(format!("skill already exists: {}", entry.skill_id));
    }
    let temporary_destination = destination.with_file_name(format!(".{}.tmp", entry.skill_id));
    if temporary_destination.exists() {
        fs::remove_dir_all(&temporary_destination).map_err(|error| {
            format!(
                "failed to remove stale temporary skill directory {}: {error}",
                temporary_destination.display()
            )
        })?;
    }
    copy_dir_all(&source, &temporary_destination)?;
    let installed_files = relative_package_files(&temporary_destination)?;
    write_remote_source_metadata(
        &temporary_destination,
        &entry,
        &source_index,
        &installed_files,
    )?;
    if destination.exists() {
        fs::remove_dir_all(&destination)
            .map_err(|error| format!("failed to replace {}: {error}", destination.display()))?;
    }
    fs::rename(&temporary_destination, &destination).map_err(|error| {
        format!(
            "failed to move {} to {}: {error}",
            temporary_destination.display(),
            destination.display()
        )
    })?;

    sync_skills_index(
        destination_root,
        &entry.skill_id,
        &format!("{}/SKILL.md", entry.skill_id),
    )?;
    let mut extra = BTreeMap::new();
    extra.insert("source_path".to_owned(), json!(entry.path));
    extra.insert("installed_files".to_owned(), json!(installed_files));
    append_skill_operation(
        destination_root,
        "install_remote",
        &entry.skill_id,
        &source_index,
        &destination.display().to_string(),
        extra,
    )?;
    Ok(RemoteInstallResult {
        skill_id: entry.skill_id,
        destination,
        source_index,
    })
}

fn load_remote_index(paths: &WorkspacePaths) -> Result<(String, Option<PathBuf>, String), String> {
    if let Some(source) = env::var_os(REMOTE_SKILLS_INDEX_ENV) {
        let source = PathBuf::from(source);
        let text = fs::read_to_string(&source)
            .map_err(|error| format!("failed to read remote skills index fixture: {error}"))?;
        let display = source.display().to_string();
        return Ok((text, Some(source), display));
    }
    let cached = paths.skills_dir.join(REMOTE_SKILLS_INDEX_FILENAME);
    if cached.is_file() {
        let text = fs::read_to_string(&cached)
            .map_err(|error| format!("failed to read cached remote skills index: {error}"))?;
        let display = cached.display().to_string();
        return Ok((text, Some(cached), display));
    }
    Err(format!(
        "remote skill index source is unavailable; run `millrace skills refresh-remote-index` with {REMOTE_SKILLS_INDEX_ENV} set"
    ))
}

#[derive(Debug, Clone)]
struct RemoteSkillEntry {
    skill_id: String,
    path: String,
    status: String,
}

fn parse_remote_skill_index(index_text: &str) -> Vec<RemoteSkillEntry> {
    let mut entries = Vec::new();
    for line in index_text.lines() {
        let stripped = line.trim();
        if !stripped.starts_with('|') || !stripped.contains('`') {
            continue;
        }
        let columns = split_markdown_table_row(stripped);
        if columns.len() < 5
            || columns[0].eq_ignore_ascii_case("skill")
            || columns[0].chars().all(|ch| ch == '-' || ch == ' ')
        {
            continue;
        }
        let skill_id = strip_markdown_code(&columns[0]).trim().to_owned();
        let path = strip_markdown_code(&columns[3]).trim().to_owned();
        let status = strip_markdown_code(&columns[4]).trim().to_ascii_lowercase();
        if !skill_id.is_empty() && !path.is_empty() {
            entries.push(RemoteSkillEntry {
                skill_id,
                path,
                status,
            });
        }
    }
    entries
}

fn source_directory_for_remote_entry(entry: &RemoteSkillEntry) -> Result<PathBuf, String> {
    let path = Path::new(&entry.path);
    reject_unsafe_relative_path(path, "remote skill path")?;
    if !path.file_name().is_some_and(|name| name == "SKILL.md") {
        return Err(format!(
            "remote skill path must point to SKILL.md: {}",
            entry.path
        ));
    }
    path.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("remote skill path must include a parent: {}", entry.path))
}

fn remote_mirror_root(index_source: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(root) = env::var_os(REMOTE_SKILLS_ROOT_ENV) {
        let root = PathBuf::from(root);
        if root.is_dir() {
            return Ok(root);
        }
        return Err(format!(
            "remote skill mirror root is not a directory: {}",
            root.display()
        ));
    }
    index_source
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            format!("remote skill mirror root is unavailable; set {REMOTE_SKILLS_ROOT_ENV}")
        })
}

fn write_remote_source_metadata(
    destination: &Path,
    entry: &RemoteSkillEntry,
    source_index: &str,
    installed_files: &[String],
) -> Result<(), String> {
    let mut file_hashes = BTreeMap::new();
    for relative_path in installed_files {
        let path = destination.join(relative_path);
        let bytes = fs::read(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        file_hashes.insert(relative_path.clone(), hex_string(Sha256::digest(&bytes)));
    }
    let payload = json!({
        "schema_version": "1.0",
        "kind": "remote_skill_source",
        "skill_id": entry.skill_id,
        "source_index_url": source_index,
        "source_tree_url": "file-backed",
        "source_tree_sha": Value::Null,
        "source_path": entry.path,
        "installed_files": installed_files,
        "file_sha256": file_hashes,
        "installed_at": timestamp_now()?,
    });
    let path = destination.join("remote_source.json");
    let encoded = serde_json::to_string_pretty(&payload).map_err(|error| error.to_string())?;
    atomic_write_text(&path, &(encoded + "\n")).map_err(|error| error.to_string())
}

fn split_markdown_table_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|column| column.trim().to_owned())
        .collect()
}

fn strip_markdown_code(value: &str) -> String {
    value.replace('`', "")
}

fn reject_unsafe_relative_path(path: &Path, label: &str) -> Result<(), String> {
    if path.is_absolute() {
        return Err(format!("unsafe {label}: {}", path.display()));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(format!("unsafe {label}: {}", path.display()));
            }
        }
    }
    Ok(())
}

fn require_learning_mode(paths: &WorkspacePaths, mode_id: Option<&str>) -> Result<(), String> {
    let resolved = resolve_compile_assets(paths, mode_id).map_err(|error| error.to_string())?;
    if resolved
        .mode
        .loop_ids_by_plane
        .contains_key(&Plane::Learning)
    {
        Ok(())
    } else {
        Err("current mode does not enable the learning plane".to_owned())
    }
}

fn learning_request_document(
    requested_action: LearningRequestAction,
    title: &str,
    summary: &str,
    target_skill_id: Option<String>,
) -> Result<LearningRequestDocument, String> {
    let created_at = timestamp_now()?;
    Ok(LearningRequestDocument {
        learning_request_id: learning_request_id(title, summary, &created_at),
        title: title.to_owned(),
        summary: summary.to_owned(),
        requested_action,
        target_skill_id,
        target_stage: None,
        source_refs: Vec::new(),
        preferred_output_paths: Vec::new(),
        trigger_metadata: json!({}),
        originating_run_ids: Vec::new(),
        artifact_paths: Vec::new(),
        references: Vec::new(),
        created_at: Timestamp::parse("created_at", &created_at)
            .map_err(|error| error.to_string())?,
        created_by: "millrace skills".to_owned(),
        updated_at: None,
    })
}

fn learning_request_id(title: &str, summary: &str, created_at: &str) -> String {
    let payload = format!("{title}\n{summary}\n{created_at}\n{}", process::id());
    format!(
        "learn-{}",
        hex_prefix(Sha256::digest(payload.as_bytes()), 12)
    )
}

fn enqueue_learning_request(
    paths: &WorkspacePaths,
    document: &LearningRequestDocument,
) -> Result<Vec<String>, String> {
    let destination = QueueStore::from_paths(paths.clone())
        .enqueue_learning_request(document)
        .map_err(|error| error.to_string())?;
    Ok(vec![format!(
        "queued_learning_request: {}",
        destination.display()
    )])
}

fn timestamp_now() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| error.to_string())
}

fn export_archive_path(paths: &WorkspacePaths, skill_id: &str, output: Option<&str>) -> PathBuf {
    let base = output
        .map(|value| PathBuf::from(value).with_extension(""))
        .unwrap_or_else(|| paths.root.join(skill_id));
    base.with_extension("zip")
}

#[derive(Debug)]
struct ZipEntry {
    name: String,
    data: Vec<u8>,
    crc32: u32,
    local_header_offset: u32,
}

fn write_zip_archive(source: &Path, archive: &Path) -> Result<(), String> {
    if let Some(parent) = archive.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let mut file = fs::File::create(archive)
        .map_err(|error| format!("failed to create {}: {error}", archive.display()))?;
    let mut entries = Vec::new();
    for relative_path in relative_package_files(source)? {
        let path = source.join(&relative_path);
        let data = fs::read(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let name = relative_path.replace('\\', "/");
        let crc32 = crc32(&data);
        let local_header_offset = checked_u32(zip_position(&mut file)?, "zip offset")?;
        write_local_file_header(&mut file, &name, crc32, data.len())?;
        file.write_all(&data)
            .map_err(|error| format!("failed to write {}: {error}", archive.display()))?;
        entries.push(ZipEntry {
            name,
            data,
            crc32,
            local_header_offset,
        });
    }
    let central_directory_offset = checked_u32(zip_position(&mut file)?, "zip offset")?;
    for entry in &entries {
        write_central_directory_header(&mut file, entry)?;
    }
    let central_directory_size = checked_u32(
        zip_position(&mut file)? - u64::from(central_directory_offset),
        "zip size",
    )?;
    write_end_of_central_directory(
        &mut file,
        checked_u16(entries.len(), "zip entry count")?,
        central_directory_size,
        central_directory_offset,
    )?;
    Ok(())
}

fn zip_position(file: &mut fs::File) -> Result<u64, String> {
    file.stream_position()
        .map_err(|error| format!("failed to inspect zip archive position: {error}"))
}

fn relative_package_files(source: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    collect_relative_files(source, source, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_relative_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<String>,
) -> Result<(), String> {
    for entry in sorted_read_dir(directory)? {
        let path = entry.path();
        if path.is_dir() {
            collect_relative_files(root, &path, files)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            files.push(relative);
        }
    }
    Ok(())
}

fn write_local_file_header(
    file: &mut fs::File,
    name: &str,
    crc32: u32,
    size: usize,
) -> Result<(), String> {
    let name_bytes = name.as_bytes();
    write_u32(file, 0x0403_4b50)?;
    write_u16(file, 20)?;
    write_u16(file, 0)?;
    write_u16(file, 0)?;
    write_u16(file, 0)?;
    write_u16(file, 0)?;
    write_u32(file, crc32)?;
    write_u32(file, checked_u32(size as u64, "zip file size")?)?;
    write_u32(file, checked_u32(size as u64, "zip file size")?)?;
    write_u16(file, checked_u16(name_bytes.len(), "zip filename length")?)?;
    write_u16(file, 0)?;
    file.write_all(name_bytes)
        .map_err(|error| format!("failed to write zip header: {error}"))
}

fn write_central_directory_header(file: &mut fs::File, entry: &ZipEntry) -> Result<(), String> {
    let name_bytes = entry.name.as_bytes();
    write_u32(file, 0x0201_4b50)?;
    write_u16(file, 20)?;
    write_u16(file, 20)?;
    write_u16(file, 0)?;
    write_u16(file, 0)?;
    write_u16(file, 0)?;
    write_u16(file, 0)?;
    write_u32(file, entry.crc32)?;
    write_u32(file, checked_u32(entry.data.len() as u64, "zip file size")?)?;
    write_u32(file, checked_u32(entry.data.len() as u64, "zip file size")?)?;
    write_u16(file, checked_u16(name_bytes.len(), "zip filename length")?)?;
    write_u16(file, 0)?;
    write_u16(file, 0)?;
    write_u16(file, 0)?;
    write_u16(file, 0)?;
    write_u32(file, 0)?;
    write_u32(file, entry.local_header_offset)?;
    file.write_all(name_bytes)
        .map_err(|error| format!("failed to write zip directory: {error}"))
}

fn write_end_of_central_directory(
    file: &mut fs::File,
    entry_count: u16,
    central_directory_size: u32,
    central_directory_offset: u32,
) -> Result<(), String> {
    write_u32(file, 0x0605_4b50)?;
    write_u16(file, 0)?;
    write_u16(file, 0)?;
    write_u16(file, entry_count)?;
    write_u16(file, entry_count)?;
    write_u32(file, central_directory_size)?;
    write_u32(file, central_directory_offset)?;
    write_u16(file, 0)
}

fn write_u16(file: &mut fs::File, value: u16) -> Result<(), String> {
    file.write_all(&value.to_le_bytes())
        .map_err(|error| format!("failed to write zip archive: {error}"))
}

fn write_u32(file: &mut fs::File, value: u32) -> Result<(), String> {
    file.write_all(&value.to_le_bytes())
        .map_err(|error| format!("failed to write zip archive: {error}"))
}

fn checked_u16(value: usize, label: &str) -> Result<u16, String> {
    u16::try_from(value).map_err(|_| format!("{label} exceeds ZIP32 limit"))
}

fn checked_u32(value: u64, label: &str) -> Result<u32, String> {
    u32::try_from(value).map_err(|_| format!("{label} exceeds ZIP32 limit"))
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn hex_prefix(bytes: impl AsRef<[u8]>, len: usize) -> String {
    let encoded = hex_string(bytes);
    encoded.chars().take(len).collect()
}

fn hex_string(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
