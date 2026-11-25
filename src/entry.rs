//! Command entry of the `ladon` program: argument parsing, dispatch and the commands that
//! reach the operating system. Every external effect is reached through the [`Host`] trait.
use crate::{
    agent_api::{self, AgentApi},
    doctask_cli::{self, DocArgs},
    engine::{normalise_relative, Engine},
    google_oauth,
    model::FileRecord,
    nema,
    policy::{self, is_root_rule_event, needs_full_scan, LadonPolicy},
    provider::provider_from_env,
    search, shell,
    shell::ShellResult,
    tagging,
    workspace::FileSpace,
};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use clap::{Parser, Subcommand};
use notify::{RecursiveMode, Watcher};
use similar::{ChangeTag, TextDiff};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

/// Command-line arguments of `ladon`.
#[derive(Parser)]
#[command(
    name = "ladon",
    version,
    about = "LADON Explorer: event-first document history kernel"
)]
pub struct Cli {
    #[arg(long, global = true, default_value = ".")]
    pub root: PathBuf,
    #[command(subcommand)]
    pub cmd: Command,
}

/// The subcommands of `ladon`.
#[derive(Subcommand)]
pub enum Command {
    Init,
    Scan {
        #[arg(long, default_value = "human")]
        actor: String,
    },
    Watch {
        #[arg(long, default_value = "watcher")]
        actor: String,
    },
    Capture {
        file: PathBuf,
        #[arg(long, default_value = "human")]
        actor: String,
        #[arg(long, default_value = "manual capture")]
        reason: String,
    },
    Files,
    History {
        file: PathBuf,
    },
    Verify {
        file: PathBuf,
    },
    Diff {
        from: String,
        to: String,
    },
    IgnoreCheck {
        file: PathBuf,
    },
    Grep {
        regex: String,
        #[arg(long, default_value_t = 40)]
        limit: usize,
    },
    TagFolder {
        #[arg(long, default_value = "topic,project,stage")]
        presets: String,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long)]
        endpoint: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        recursive: bool,
    },
    Tags {
        file: PathBuf,
    },
    Annotate {
        file: PathBuf,
        #[arg(long, default_value = "ROOT")]
        section: String,
        text: String,
        #[arg(long, default_value = "human")]
        actor: String,
    },
    Task {
        file: PathBuf,
        #[arg(long, default_value = "ROOT")]
        section: String,
        title: String,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long, default_value = "human")]
        actor: String,
    },
    /// Document-task commands: `ladon doc <command>`.
    Doc(DocArgs),
    Plan {
        version_id: String,
    },
    Restore {
        version_id: String,
        #[arg(long)]
        approve: bool,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        destination: Option<PathBuf>,
    },
    Archive {
        version_id: String,
        #[arg(long, default_value = "local")]
        provider: String,
        #[arg(long)]
        destination: Option<PathBuf>,
        #[arg(long)]
        evict_local: bool,
    },
    Parallel {
        graph_file: PathBuf,
    },
    AgentCapabilities,
    AgentList {
        folder: PathBuf,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    SensitiveAdd {
        file: PathBuf,
    },
    ShellMenu {
        file: PathBuf,
    },
    #[command(hide = true)]
    ShellHostInternal {
        file: PathBuf,
    },
    FileSpace,
    /// Signs this user in to Google Drive. The refresh token is stored in protected form.
    GoogleLogin,
    /// Removes the stored Google Drive sign-in.
    GoogleLogout,
}

/// A failure that has already been printed. `main_entry` exits with `code` and prints nothing more.
#[derive(Debug)]
pub struct Reported {
    pub code: i32,
}

impl std::fmt::Display for Reported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "command failed with status {}", self.code)
    }
}

impl std::error::Error for Reported {}

/// One watch event source: the events of a watcher, ending with disconnection.
pub trait WatchSource {
    /// Waits for the next event; `None` when the watcher has disconnected.
    fn recv(&mut self) -> Option<notify::Result<notify::Event>>;
    /// Returns a queued event without waiting; `None` when no event is queued.
    fn try_recv(&mut self) -> Option<notify::Result<notify::Event>>;
}

/// The external effects of the command entry.
#[async_trait(?Send)]
pub trait Host: Sync {
    /// Starts watching the workspace recursively.
    fn start_watch(&self, root: &Path) -> Result<Box<dyn WatchSource>>;
    /// Reports whether `watch` must stop before it reads the next event.
    fn stop_requested(&self) -> bool {
        false
    }
    /// Opens the Google sign-in page in the browser.
    fn open_browser(&self, url: &str);
    /// The folder that stores the Google sign-in, if one is available.
    fn settings_dir(&self) -> Option<PathBuf>;
    /// Runs the Google sign-in; `on_url` receives the authorization URL.
    async fn oauth_login<F: Fn(&str) + Sync>(
        &self,
        on_url: &F,
        timeout: Duration,
    ) -> Result<String>;
    fn save_refresh_token(&self, settings_dir: &Path, refresh_token: &str) -> Result<()>;
    fn delete_refresh_token(&self, settings_dir: &Path) -> Result<()>;
    /// Asks a model endpoint for tags of one file.
    async fn ai_tags(
        &self,
        endpoint: &str,
        model: &str,
        description: &str,
        filename: &str,
        preview: &str,
    ) -> Result<Vec<String>>;
    /// Runs the shell menu host in a separate process and returns its exit status.
    fn spawn_shell_host(&self, root: &Path, path: &Path) -> Result<i32>;
    fn open_in_explorer(&self, path: &Path) -> Result<()>;
    /// Runs the shell host branch for a file in the current process.
    fn run_isolated_host(&self, path: &Path) -> Result<ShellResult>;
}

/// The real external effects.
pub struct SystemHost;

/// Watch source backed by a recommended filesystem watcher.
struct SystemWatch {
    _watcher: notify::RecommendedWatcher,
    events: mpsc::Receiver<notify::Result<notify::Event>>,
}

impl WatchSource for SystemWatch {
    fn recv(&mut self) -> Option<notify::Result<notify::Event>> {
        self.events.recv().ok()
    }

    fn try_recv(&mut self) -> Option<notify::Result<notify::Event>> {
        self.events.try_recv().ok()
    }
}

#[async_trait(?Send)]
impl Host for SystemHost {
    fn start_watch(&self, root: &Path) -> Result<Box<dyn WatchSource>> {
        let (tx, events) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(move |result| {
            let _ = tx.send(result);
        })?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        Ok(Box::new(SystemWatch {
            _watcher: watcher,
            events,
        }))
    }

    // rundll32 receives the address as one argument, so the query string is not split by a shell.
    fn open_browser(&self, url: &str) {
        #[cfg(windows)]
        let _ = std::process::Command::new("rundll32.exe")
            .arg("url.dll,FileProtocolHandler")
            .arg(url)
            .spawn();
        #[cfg(not(windows))]
        let _ = url;
    }

    fn settings_dir(&self) -> Option<PathBuf> {
        google_oauth::settings_dir()
    }

    async fn oauth_login<F: Fn(&str) + Sync>(
        &self,
        on_url: &F,
        timeout: Duration,
    ) -> Result<String> {
        let client = google_oauth::OAuthClient::from_env()?;
        google_oauth::login(&client, |url: &str| on_url(url), timeout).await
    }

    fn save_refresh_token(&self, settings_dir: &Path, refresh_token: &str) -> Result<()> {
        google_oauth::save_refresh_token(settings_dir, refresh_token)
    }

    fn delete_refresh_token(&self, settings_dir: &Path) -> Result<()> {
        google_oauth::delete_refresh_token(settings_dir)
    }

    async fn ai_tags(
        &self,
        endpoint: &str,
        model: &str,
        description: &str,
        filename: &str,
        preview: &str,
    ) -> Result<Vec<String>> {
        tagging::ai_tags(endpoint, model, description, filename, preview).await
    }

    fn spawn_shell_host(&self, root: &Path, path: &Path) -> Result<i32> {
        shell::spawn_shell_host(root, path)
    }

    fn open_in_explorer(&self, path: &Path) -> Result<()> {
        shell::open_in_explorer(path)
    }

    fn run_isolated_host(&self, path: &Path) -> Result<ShellResult> {
        shell::run_isolated_host(path)
    }
}

/// Stack size of the thread that parses the arguments and runs the command. The command tree of
/// `clap` and the futures of the commands need more than the default main-thread stack.
const COMMAND_STACK_BYTES: usize = 32 * 1024 * 1024;

/// Parses the process arguments, runs the command on the system and returns its result.
/// Parsing and running happen on a thread with [`COMMAND_STACK_BYTES`] of stack; a panic on that
/// thread is propagated to the caller unchanged.
pub fn main_entry() -> Result<()> {
    let worker = std::thread::Builder::new()
        .name("ladon-command".to_string())
        .stack_size(COMMAND_STACK_BYTES)
        .spawn(run_process)?;
    match worker.join() {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Parses the process arguments, runs the command on a runtime and maps a [`Reported`] failure to
/// the process exit code.
fn run_process() -> Result<()> {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(run(cli, &SystemHost));
    if let Err(err) = &result {
        if let Some(reported) = err.downcast_ref::<Reported>() {
            std::process::exit(reported.code);
        }
    }
    result
}

/// Runs one parsed command line against the given host.
pub async fn run<H: Host>(cli: Cli, host: &H) -> Result<()> {
    let root = cli.root;
    match cli.cmd {
        Command::ShellMenu { file } => shell_menu(host, &root, &file),
        Command::ShellHostInternal { file } => shell_host_internal(host, &root, &file).await,
        Command::Init => {
            let engine = Engine::init(&root).await?;
            println!("Initialized: {}", engine.root.display());
            Ok(())
        }
        Command::GoogleLogin => google_login(host).await,
        Command::GoogleLogout => google_logout(host),
        cmd => {
            let engine = Engine::open(&root).await?;
            command(host, &engine, cmd).await
        }
    }
}

/// Runs a command that operates on an open workspace.
async fn command<H: Host>(host: &H, engine: &Engine, cmd: Command) -> Result<()> {
    match cmd {
        Command::AgentCapabilities => {
            println!(
                "{}",
                serde_json::to_string_pretty(&agent_api::capabilities())?
            );
        }
        Command::AgentList { folder, limit } => {
            let api = AgentApi { engine };
            println!(
                "{}",
                serde_json::to_string_pretty(&api.list_folder(&folder, limit).await?)?
            );
        }
        Command::SensitiveAdd { file } => {
            let rule = policy::add_sensitive(&engine.root, &file)?;
            engine.reload_policy()?;
            println!("Updated sensitivity rules: {}", rule.display());
        }
        Command::Scan { actor } => {
            let results = engine.scan(&actor).await?;
            let changed = results.iter().filter(|r| r.status == "captured").count();
            println!("scanned {}, captured {} version(s)", results.len(), changed);
        }
        Command::Capture {
            file,
            actor,
            reason,
        } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&engine.capture(&file, &actor, &reason).await?)?
            );
        }
        Command::Watch { actor } => watch(host, engine, &actor).await?,
        Command::Files => println!(
            "{}",
            serde_json::to_string_pretty(&engine.store.list_files().await?)?
        ),
        Command::History { file } => {
            let record = resolve_record(engine, &file).await?;
            let versions = engine.store.versions(&record.id).await?;
            let events = engine.store.events(&record.id).await?;
            let chain_valid = engine.store.verify_chain(&record.id).await?;
            let report = serde_json::json!({"file": record, "versions": versions, "events": events, "chain_valid": chain_valid});
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Verify { file } => {
            let record = resolve_record(engine, &file).await?;
            let valid = engine.store.verify_chain(&record.id).await?;
            println!("event chain {}", if valid { "VALID" } else { "BROKEN" });
            if !valid {
                bail!("history chain damaged");
            }
        }
        Command::Diff { from, to } => {
            let left = engine.load_version(&from, None).await?;
            let right = engine.load_version(&to, None).await?;
            let a = String::from_utf8_lossy(&left);
            let b = String::from_utf8_lossy(&right);
            let diff = TextDiff::from_lines(a.as_ref(), b.as_ref());
            for (i, change) in diff.iter_all_changes().enumerate() {
                if i > 800 {
                    println!("[diff truncated]");
                    break;
                }
                let tag = match change.tag() {
                    ChangeTag::Delete => '-',
                    ChangeTag::Insert => '+',
                    ChangeTag::Equal => ' ',
                };
                print!("{}{}", tag, change);
            }
        }
        Command::IgnoreCheck { file } => {
            let policy = LadonPolicy::load(&engine.root)?;
            let excluded = policy.excluded(&file)?;
            println!(
                "{}",
                if excluded {
                    "EXCLUDED: local only"
                } else {
                    "ALLOWED: subject to explicit archive action"
                }
            );
        }
        Command::Grep { regex, limit } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&search::grep(&engine.root, &regex, limit)?)?
            );
        }
        Command::TagFolder {
            presets,
            description,
            endpoint,
            model,
            recursive,
        } => {
            let options = TagOptions {
                presets: &presets,
                description: &description,
                endpoint: endpoint.as_deref(),
                model: model.as_deref(),
                recursive,
            };
            let count = tag_folder(host, engine, &options).await?;
            println!("tagged {} file(s)", count);
        }
        Command::Tags { file } => {
            let record = resolve_record(engine, &file).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&engine.store.tags(&record.id).await?)?
            );
        }
        Command::Annotate {
            file,
            section,
            text,
            actor,
        } => {
            let file = file.to_string_lossy().into_owned();
            let code = doctask_cli::annotate_legacy(
                engine,
                &actor,
                &file,
                &section,
                &text,
                &mut doctask_cli::SystemIo,
            )
            .await?;
            if code != 0 {
                return Err(Reported { code }.into());
            }
        }
        Command::Task {
            file,
            section,
            title,
            description,
            actor,
        } => {
            let file = file.to_string_lossy().into_owned();
            let code = doctask_cli::task_legacy(
                engine,
                &actor,
                &file,
                &section,
                &title,
                &description,
                &mut doctask_cli::SystemIo,
            )
            .await?;
            if code != 0 {
                return Err(Reported { code }.into());
            }
        }
        Command::Doc(args) => {
            let code = Box::pin(doctask_cli::run_doc(
                engine,
                &args,
                &mut doctask_cli::SystemIo,
            ))
            .await?;
            if code != 0 {
                return Err(Reported { code }.into());
            }
        }
        Command::Plan { version_id } => println!("{}", engine.restore_plan(&version_id).await?),
        Command::Restore {
            version_id,
            approve,
            provider,
            destination,
        } => {
            let adapter = match provider {
                Some(provider) => Some(provider_from_env(&provider, destination.as_deref())?),
                None => None,
            };
            let object = engine
                .restore(&version_id, approve, adapter.as_deref())
                .await?;
            println!("{}", serde_json::to_string_pretty(&object)?);
        }
        Command::Archive {
            version_id,
            provider,
            destination,
            evict_local,
        } => {
            let adapter = provider_from_env(&provider, destination.as_deref())?;
            let object = engine
                .archive_version(&version_id, adapter.as_ref(), evict_local)
                .await?;
            println!("{}", serde_json::to_string_pretty(&object)?);
        }
        Command::Parallel { graph_file } => {
            let data = fs::read_to_string(graph_file)?;
            let edges = nema::parse_graph_edges(&data)?;
            let nodes: HashSet<String> = data
                .lines()
                .filter(|line| !line.starts_with(' ') && !line.is_empty())
                .filter_map(|line| {
                    let token = line.split(' ').next()?;
                    if token.contains('>') || token.contains('<') || token.contains('=') {
                        None
                    } else {
                        Some(token.to_string())
                    }
                })
                .collect();
            let groups = nema::topo_batches(&nodes.into_iter().collect::<Vec<_>>(), &edges)?;
            println!("{}", serde_json::to_string_pretty(&groups)?);
        }
        Command::FileSpace => {
            let files = engine.store.list_files().await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&FileSpace::from_files(&files))?
            );
        }
        Command::Init
        | Command::ShellMenu { .. }
        | Command::ShellHostInternal { .. }
        | Command::GoogleLogin
        | Command::GoogleLogout => unreachable!("handled by run"),
    }
    Ok(())
}

/// Runs the shell menu: the host shows the menu, and Explorer opens the file when the host fails.
fn shell_menu<H: Host>(host: &H, root: &Path, file: &Path) -> Result<()> {
    let path = file.canonicalize()?;
    let status = host.spawn_shell_host(root, &path)?;
    if status != 0 {
        host.open_in_explorer(&path)?;
    }
    Ok(())
}

/// Runs the hidden shell host branch for one file.
async fn shell_host_internal<H: Host>(host: &H, root: &Path, file: &Path) -> Result<()> {
    let path = file.canonicalize()?;
    match host.run_isolated_host(&path) {
        Ok(ShellResult::ExecutedOrDismissed) => Ok(()),
        Ok(ShellResult::MarkSensitive) => {
            let engine = Engine::open(root).await?;
            policy::add_sensitive(&engine.root, &path)?;
            println!("Sensitive rule added for {}", path.display());
            Ok(())
        }
        Ok(ShellResult::ViewHistory) => {
            println!("Use: ladon --root <workspace> history <file> (GUI pane is planned)");
            Ok(())
        }
        Ok(ShellResult::DocumentTask) => {
            println!("Open LADON Document Task editor (GUI integration planned)");
            Ok(())
        }
        Ok(ShellResult::OpenExplorer) => host.open_in_explorer(&path),
        Err(err) => {
            eprintln!("{err:#}");
            Err(Reported { code: 1 }.into())
        }
    }
}

/// Signs the user in and stores the refresh token. The authorization URL is never printed.
async fn google_login<H: Host>(host: &H) -> Result<()> {
    let dir = host
        .settings_dir()
        .context("no settings folder: set LOCALAPPDATA or LADON_SETTINGS_DIR")?;
    let open = |url: &str| {
        println!("Complete the Google Drive sign-in in the browser. Waiting up to five minutes.");
        host.open_browser(url);
    };
    let refresh = host.oauth_login(&open, Duration::from_secs(300)).await?;
    host.save_refresh_token(&dir, &refresh)?;
    println!("Google Drive sign-in saved");
    Ok(())
}

/// Removes the stored Google sign-in.
fn google_logout<H: Host>(host: &H) -> Result<()> {
    let dir = host
        .settings_dir()
        .context("no settings folder: set LOCALAPPDATA or LADON_SETTINGS_DIR")?;
    host.delete_refresh_token(&dir)?;
    println!("Google Drive sign-in removed");
    Ok(())
}

/// Resolves a path given on the command line to its tracked record.
async fn resolve_record(engine: &Engine, path: &Path) -> Result<FileRecord> {
    let rel = if path.is_absolute() {
        engine.relative_for_input(path)?
    } else {
        normalise_relative(&path.to_string_lossy())
    };
    engine.file_by_path(&rel).await
}

/// The options of `tag-folder`.
struct TagOptions<'a> {
    presets: &'a str,
    description: &'a str,
    endpoint: Option<&'a str>,
    model: Option<&'a str>,
    recursive: bool,
}

/// Tags the tracked files of the workspace and returns how many files received tags.
async fn tag_folder<H: Host>(host: &H, engine: &Engine, options: &TagOptions<'_>) -> Result<usize> {
    let policy = LadonPolicy::load(&engine.root)?;
    let mut count = 0;
    let depth = if options.recursive { 32 } else { 1 };
    let walker = walkdir::WalkDir::new(&engine.root)
        .max_depth(depth)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0 || !matches!(e.file_name().to_str(), Some(".ladon" | ".git"))
        })
        .filter_map(std::result::Result::ok);
    for entry in walker {
        if !entry.file_type().is_file() {
            continue;
        }
        if entry
            .path()
            .components()
            .any(|s| s.as_os_str() == ".git" || s.as_os_str() == ".ladon")
            || entry.file_name() == ".LADONignore"
        {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(&engine.root)?
            .to_string_lossy()
            .replace('\\', "/");
        let Some(file) = engine.store.find_path(&rel).await? else {
            continue;
        };
        let mut buf = Vec::new();
        let mut handle = fs::File::open(entry.path())?;
        handle.by_ref().take(16384).read_to_end(&mut buf)?;
        let content = String::from_utf8_lossy(&buf);
        let mut tags = tagging::heuristic_tags(&rel, &content);
        if let (Some(url), Some(model)) = (options.endpoint, options.model) {
            // Sensitive files, and copies of their content at other paths, may only be sent to a loopback local model if explicitly requested.
            let excluded = policy.excluded(entry.path())?
                || engine.content_is_excluded(&file.current_hash).await?;
            if excluded && !tagging::local_endpoint(url)? {
                eprintln!("skipped remote AI tagging for excluded file: {}", rel);
            } else {
                let description =
                    format!("{}; dimensions: {}", options.description, options.presets);
                tags.extend(
                    host.ai_tags(url, model, &description, &rel, &content)
                        .await?,
                );
            }
        }
        if !options.description.is_empty() {
            tags.push("rule-described".into());
        }
        tags.sort();
        tags.dedup();
        if !tags.is_empty() {
            let source = if options.endpoint.is_some() && options.model.is_some() {
                "ai+rules"
            } else {
                "heuristic"
            };
            engine.store.set_tags(&file.id, &tags, source, "v1").await?;
            count += 1;
        }
    }
    Ok(count)
}

/// Runs the watch loop until the host requests a stop or the watcher fails.
async fn watch<H: Host>(host: &H, engine: &Engine, actor: &str) -> Result<()> {
    let mut source = host.start_watch(&engine.root)?;
    println!("watching {} — Ctrl+C stops", engine.root.display());
    loop {
        if host.stop_requested() {
            return Ok(());
        }
        let event = source
            .recv()
            .ok_or_else(|| anyhow!("watcher disconnected"))??;
        let mut paths: HashSet<PathBuf> = event.paths.into_iter().collect();
        std::thread::sleep(Duration::from_millis(350));
        while let Some(next) = source.try_recv() {
            paths.extend(next?.paths);
        }
        process_batch(engine, actor, paths).await?;
    }
}

/// Handles one batch of changed paths: captures files, marks removals and runs scans when needed.
async fn process_batch(engine: &Engine, actor: &str, paths: HashSet<PathBuf>) -> Result<()> {
    let mut full_scan = false;
    for path in paths {
        if path
            .components()
            .any(|c| c.as_os_str() == ".ladon" || c.as_os_str() == ".git")
        {
            continue;
        }
        if is_root_rule_event(&engine.root, &path)
            || path
                .file_name()
                .map(|s| s == ".LADONignore")
                .unwrap_or(false)
        {
            if let Err(err) = engine.reload_policy() {
                eprintln!("policy reload failed, check external-transfer protections: {err:#}");
            } else {
                println!("policy reloaded: {}", path.display());
            }
            continue;
        }
        let is_file = path.is_file();
        full_scan |= needs_full_scan(&path, is_file, &engine.root);
        if is_file {
            if let Err(e) = engine.capture(&path, actor, "filesystem stable save").await {
                eprintln!("capture warning {}: {e:#}", path.display());
            }
        } else if let Ok(relative) = path.strip_prefix(&engine.root) {
            let rel = relative.to_string_lossy().replace('\\', "/");
            if let Some(record) = engine.store.find_path(&rel).await? {
                if let Err(e) = engine
                    .store
                    .mark_deleted(&record, actor, "watcher removal")
                    .await
                {
                    eprintln!("delete warning: {e:#}");
                }
            }
        }
    }
    if full_scan {
        match engine.scan(actor).await {
            Ok(results) => println!("full scan after watch event: scanned {}", results.len()),
            Err(err) => eprintln!("scan warning: {err:#}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Mutex};

    /// A host whose external effects are scripted and recorded.
    #[derive(Default)]
    struct FakeHost {
        stop: bool,
        watch_error: Option<String>,
        watch_events: Mutex<VecDeque<notify::Result<notify::Event>>>,
        settings: Option<PathBuf>,
        login: Option<std::result::Result<String, String>>,
        logins: Mutex<usize>,
        browser: Mutex<Vec<String>>,
        saved: Mutex<Vec<(PathBuf, String)>>,
        deleted: Mutex<Vec<PathBuf>>,
        ai_reply: Vec<String>,
        ai_calls: Mutex<Vec<String>>,
        shell_status: i32,
        shell_spawns: Mutex<usize>,
        shell: Option<fn() -> Result<ShellResult>>,
        explorer: Mutex<Vec<PathBuf>>,
    }

    struct FakeWatch(VecDeque<notify::Result<notify::Event>>);

    impl WatchSource for FakeWatch {
        fn recv(&mut self) -> Option<notify::Result<notify::Event>> {
            self.0.pop_front()
        }

        fn try_recv(&mut self) -> Option<notify::Result<notify::Event>> {
            self.0.pop_front()
        }
    }

    #[async_trait(?Send)]
    impl Host for FakeHost {
        fn start_watch(&self, _root: &Path) -> Result<Box<dyn WatchSource>> {
            if let Some(message) = &self.watch_error {
                bail!("{message}");
            }
            let events = std::mem::take(&mut *self.watch_events.lock().unwrap());
            Ok(Box::new(FakeWatch(events)))
        }

        fn stop_requested(&self) -> bool {
            self.stop
        }

        fn open_browser(&self, url: &str) {
            self.browser.lock().unwrap().push(url.to_string());
        }

        fn settings_dir(&self) -> Option<PathBuf> {
            self.settings.clone()
        }

        async fn oauth_login<F: Fn(&str) + Sync>(
            &self,
            on_url: &F,
            _timeout: Duration,
        ) -> Result<String> {
            *self.logins.lock().unwrap() += 1;
            on_url("https://accounts.example.test/o/oauth2/auth?state=fixture");
            match &self.login {
                Some(Ok(token)) => Ok(token.clone()),
                Some(Err(message)) => bail!("{message}"),
                None => bail!("login not scripted"),
            }
        }

        fn save_refresh_token(&self, settings_dir: &Path, refresh_token: &str) -> Result<()> {
            self.saved
                .lock()
                .unwrap()
                .push((settings_dir.to_path_buf(), refresh_token.to_string()));
            Ok(())
        }

        fn delete_refresh_token(&self, settings_dir: &Path) -> Result<()> {
            self.deleted
                .lock()
                .unwrap()
                .push(settings_dir.to_path_buf());
            Ok(())
        }

        async fn ai_tags(
            &self,
            _endpoint: &str,
            _model: &str,
            _description: &str,
            filename: &str,
            _preview: &str,
        ) -> Result<Vec<String>> {
            self.ai_calls.lock().unwrap().push(filename.to_string());
            Ok(self.ai_reply.clone())
        }

        fn spawn_shell_host(&self, _root: &Path, _path: &Path) -> Result<i32> {
            *self.shell_spawns.lock().unwrap() += 1;
            Ok(self.shell_status)
        }

        fn open_in_explorer(&self, path: &Path) -> Result<()> {
            self.explorer.lock().unwrap().push(path.to_path_buf());
            Ok(())
        }

        fn run_isolated_host(&self, _path: &Path) -> Result<ShellResult> {
            match self.shell {
                Some(result) => result(),
                None => bail!("shell host not scripted"),
            }
        }
    }

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("ladon").chain(args.iter().copied()))
            .expect("arguments parse")
    }

    fn created(path: PathBuf) -> notify::Result<notify::Event> {
        Ok(
            notify::Event::new(notify::EventKind::Create(notify::event::CreateKind::File))
                .add_path(path),
        )
    }

    #[tokio::test]
    async fn process_batch_captures_marks_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let engine = Engine::init(root).await.unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("docs/old.md"), "old\n").unwrap();
        engine.scan("tester").await.unwrap();
        fs::remove_file(root.join("docs/old.md")).unwrap();
        fs::write(root.join("notes.txt"), "new\n").unwrap();
        fs::write(root.join(".ladon/ignored"), "x").unwrap();
        let paths: HashSet<PathBuf> = [
            root.join("notes.txt"),
            root.join("docs/old.md"),
            root.join(".LADONignore"),
            root.join(".ladon/ignored"),
        ]
        .into_iter()
        .collect();
        process_batch(&engine, "watcher", paths).await.unwrap();
        assert!(engine.store.find_path("notes.txt").await.unwrap().is_some());
        let old = engine
            .store
            .find_path("docs/old.md")
            .await
            .unwrap()
            .expect("tracked");
        assert!(old.deleted);
        assert!(engine
            .store
            .find_path(".ladon/ignored")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn watch_handles_batch_then_reports_disconnect() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let engine = Engine::init(root).await.unwrap();
        fs::write(root.join("notes.txt"), "draft\n").unwrap();
        let host = FakeHost {
            watch_events: Mutex::new(VecDeque::from([created(root.join("notes.txt"))])),
            ..Default::default()
        };
        let err = watch(&host, &engine, "watcher").await.unwrap_err();
        assert_eq!(err.to_string(), "watcher disconnected");
        assert!(engine.store.find_path("notes.txt").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn watch_returns_notify_error() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::init(dir.path()).await.unwrap();
        let host = FakeHost {
            watch_events: Mutex::new(VecDeque::from([Err(notify::Error::generic("boom"))])),
            ..Default::default()
        };
        let err = watch(&host, &engine, "watcher").await.unwrap_err();
        assert_eq!(err.to_string(), "boom");
    }

    #[tokio::test]
    async fn watch_stops_when_requested() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::init(dir.path()).await.unwrap();
        let host = FakeHost {
            stop: true,
            watch_events: Mutex::new(VecDeque::from([Err(notify::Error::generic(
                "must not be read",
            ))])),
            ..Default::default()
        };
        watch(&host, &engine, "watcher").await.unwrap();
    }

    #[tokio::test]
    async fn watch_propagates_start_error() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::init(dir.path()).await.unwrap();
        let host = FakeHost {
            watch_error: Some("start refused".into()),
            ..Default::default()
        };
        let err = watch(&host, &engine, "watcher").await.unwrap_err();
        assert_eq!(err.to_string(), "start refused");
    }

    #[tokio::test]
    async fn google_login_saves_token_after_success() {
        let settings = tempfile::tempdir().unwrap();
        let host = FakeHost {
            settings: Some(settings.path().to_path_buf()),
            login: Some(Ok("refresh-fixture".into())),
            ..Default::default()
        };
        run(parse(&["google-login"]), &host).await.unwrap();
        let saved = host.saved.lock().unwrap().clone();
        assert_eq!(
            saved,
            vec![(settings.path().to_path_buf(), "refresh-fixture".to_string())]
        );
        assert_eq!(host.browser.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn google_login_failure_saves_nothing() {
        let settings = tempfile::tempdir().unwrap();
        let host = FakeHost {
            settings: Some(settings.path().to_path_buf()),
            login: Some(Err("sign-in timed out".into())),
            ..Default::default()
        };
        let err = run(parse(&["google-login"]), &host).await.unwrap_err();
        assert_eq!(err.to_string(), "sign-in timed out");
        assert!(host.saved.lock().unwrap().is_empty());
        assert_eq!(host.browser.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn google_login_without_settings_folder() {
        let host = FakeHost::default();
        let err = run(parse(&["google-login"]), &host).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "no settings folder: set LOCALAPPDATA or LADON_SETTINGS_DIR"
        );
        assert_eq!(*host.logins.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn google_logout_is_repeatable() {
        let settings = tempfile::tempdir().unwrap();
        let host = FakeHost {
            settings: Some(settings.path().to_path_buf()),
            ..Default::default()
        };
        run(parse(&["google-logout"]), &host).await.unwrap();
        run(parse(&["google-logout"]), &host).await.unwrap();
        assert_eq!(host.deleted.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn google_logout_without_settings_folder() {
        let host = FakeHost::default();
        let err = run(parse(&["google-logout"]), &host).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "no settings folder: set LOCALAPPDATA or LADON_SETTINGS_DIR"
        );
        assert!(host.deleted.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn tag_folder_skips_excluded_files_for_remote_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let engine = Engine::init(root).await.unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("docs/req.md"), "requirement text\n").unwrap();
        fs::write(root.join("secret.txt"), "classified\n").unwrap();
        policy::add_sensitive(root, &root.join("secret.txt")).unwrap();
        engine.reload_policy().unwrap();
        engine.scan("tester").await.unwrap();
        let host = FakeHost {
            ai_reply: vec!["spec".into(), "requirements".into(), "spec".into()],
            ..Default::default()
        };
        let options = TagOptions {
            presets: "topic",
            description: "rules",
            endpoint: Some("https://ai.example.test/v1"),
            model: Some("m"),
            recursive: true,
        };
        let count = tag_folder(&host, &engine, &options).await.unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            *host.ai_calls.lock().unwrap(),
            vec!["docs/req.md".to_string()]
        );
        let req = engine.file_by_path("docs/req.md").await.unwrap();
        let req_tags = engine.store.tags(&req.id).await.unwrap();
        assert!(req_tags.iter().any(|t| t == "spec"));
        assert!(req_tags.iter().any(|t| t == "requirements"));
        assert!(req_tags.windows(2).all(|pair| pair[0] < pair[1]));
        let secret = engine.file_by_path("secret.txt").await.unwrap();
        let secret_tags = engine.store.tags(&secret.id).await.unwrap();
        assert!(secret_tags.iter().any(|t| t == "rule-described"));
        assert!(!secret_tags.iter().any(|t| t == "spec"));
    }

    #[tokio::test]
    async fn tag_folder_without_model_never_calls_ai() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let engine = Engine::init(root).await.unwrap();
        fs::write(root.join("notes.txt"), "meeting notes\n").unwrap();
        engine.scan("tester").await.unwrap();
        let host = FakeHost {
            ai_reply: vec!["spec".into()],
            ..Default::default()
        };
        let options = TagOptions {
            presets: "topic",
            description: "",
            endpoint: Some("https://ai.example.test/v1"),
            model: None,
            recursive: false,
        };
        tag_folder(&host, &engine, &options).await.unwrap();
        assert!(host.ai_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn shell_menu_opens_explorer_on_failed_host() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("doc.md");
        fs::write(&file, "text\n").unwrap();
        let file_text = file.to_string_lossy().into_owned();
        let succeeded = FakeHost {
            shell_status: 0,
            ..Default::default()
        };
        run(parse(&["shell-menu", file_text.as_str()]), &succeeded)
            .await
            .unwrap();
        assert!(succeeded.explorer.lock().unwrap().is_empty());
        let failed = FakeHost {
            shell_status: 3,
            ..Default::default()
        };
        run(parse(&["shell-menu", file_text.as_str()]), &failed)
            .await
            .unwrap();
        assert_eq!(failed.explorer.lock().unwrap().len(), 1);
        assert_eq!(*failed.shell_spawns.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn shell_host_results_are_mapped() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("doc.md");
        fs::write(&file, "text\n").unwrap();
        let file_text = file.to_string_lossy().into_owned();
        let cases: [(fn() -> Result<ShellResult>, usize); 4] = [
            (|| Ok(ShellResult::ExecutedOrDismissed), 0),
            (|| Ok(ShellResult::ViewHistory), 0),
            (|| Ok(ShellResult::DocumentTask), 0),
            (|| Ok(ShellResult::OpenExplorer), 1),
        ];
        for (result, explorer_calls) in cases {
            let host = FakeHost {
                shell: Some(result),
                ..Default::default()
            };
            run(parse(&["shell-host-internal", file_text.as_str()]), &host)
                .await
                .unwrap();
            assert_eq!(host.explorer.lock().unwrap().len(), explorer_calls);
        }
    }

    #[tokio::test]
    async fn shell_host_mark_sensitive_updates_policy() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        Engine::init(root).await.unwrap();
        let file = root.join("private.md");
        fs::write(&file, "private\n").unwrap();
        let file_text = file.to_string_lossy().into_owned();
        let host = FakeHost {
            shell: Some(|| Ok(ShellResult::MarkSensitive)),
            ..Default::default()
        };
        let root_text = root.to_string_lossy().into_owned();
        run(
            parse(&[
                "--root",
                root_text.as_str(),
                "shell-host-internal",
                file_text.as_str(),
            ]),
            &host,
        )
        .await
        .unwrap();
        let policy = LadonPolicy::load(root).unwrap();
        let canonical = file.canonicalize().unwrap();
        assert!(policy.excluded(&canonical).unwrap());
    }

    #[tokio::test]
    async fn shell_host_error_is_reported_with_code_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("doc.md");
        fs::write(&file, "text\n").unwrap();
        let file_text = file.to_string_lossy().into_owned();
        let host = FakeHost {
            shell: Some(|| Err(anyhow!("shell host crashed"))),
            ..Default::default()
        };
        let err = run(parse(&["shell-host-internal", file_text.as_str()]), &host)
            .await
            .unwrap_err();
        assert_eq!(err.downcast_ref::<Reported>().map(|r| r.code), Some(1));
    }

    #[tokio::test]
    async fn init_and_record_commands_through_run() {
        let dir = tempfile::tempdir().unwrap();
        let root_text = dir.path().to_string_lossy().into_owned();
        let host = FakeHost::default();
        run(parse(&["--root", root_text.as_str(), "init"]), &host)
            .await
            .unwrap();
        fs::write(dir.path().join("draft.md"), "draft\n").unwrap();
        let annotate = run(
            parse(&["--root", root_text.as_str(), "annotate", "draft.md", "note"]),
            &host,
        )
        .await;
        let annotate = annotate.unwrap_err();
        assert_eq!(
            annotate
                .downcast_ref::<Reported>()
                .map(|reported| reported.code),
            Some(3)
        );
        let task = run(
            parse(&[
                "--root",
                root_text.as_str(),
                "task",
                "draft.md",
                "Write summary",
            ]),
            &host,
        )
        .await;
        let task = task.unwrap_err();
        assert_eq!(
            task.downcast_ref::<Reported>()
                .map(|reported| reported.code),
            Some(3)
        );
    }

    #[test]
    fn cli_rejects_missing_and_unknown_arguments() {
        assert!(Cli::try_parse_from(["ladon"]).is_err());
        assert!(Cli::try_parse_from(["ladon", "--unknown-option", "init"]).is_err());
    }
}
