//! A fake host records every command instead of running it; no client, no
//! `claude` and no Konnect binary is ever started here.

use super::*;
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;

struct Fake {
    home: tempfile::TempDir,
    vars: HashMap<String, String>,
    calls: Vec<(String, Vec<String>)>,
    /// A call fails when its program is the first and its first or second
    /// argument the second of a pair here.
    fail: Vec<(String, String)>,
}

impl Fake {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
            vars: HashMap::new(),
            calls: Vec::new(),
            fail: Vec::new(),
        }
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.home.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.home.path().join(relative)).unwrap()
    }

    /// A package at `<home>/pkg/1-kopper`, as `kopper-package` leaves one.
    fn package(&self) -> PathBuf {
        let prefix = self.home.path().join("pkg/1-kopper");
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::fs::write(prefix.join("bin/konnect"), "stand-in").unwrap();
        std::fs::write(prefix.join(MANIFEST), r#"{"name": "1 Kopper"}"#).unwrap();
        prefix
    }

    fn calls(&self) -> Vec<String> {
        self.calls
            .iter()
            .map(|(program, args)| format!("{program} {}", args.join(" ")))
            .collect()
    }

    fn record(&self, prefix: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(rollback_file(prefix)).unwrap()).unwrap()
    }
}

impl Host for Fake {
    fn home(&self) -> &Path {
        self.home.path()
    }

    fn var(&self, name: &str) -> Option<String> {
        self.vars.get(name).cloned()
    }

    fn run(&mut self, program: &Path, args: &[String], _cwd: &Path) -> Result<(), String> {
        let program = program.to_string_lossy().into_owned();
        self.calls.push((program.clone(), args.to_vec()));
        let fails = self
            .fail
            .iter()
            .any(|(p, a)| *p == program && (args.first() == Some(a) || args.get(1) == Some(a)));
        if fails {
            Err(format!("{program} {} failed", shown(args)))
        } else {
            Ok(())
        }
    }
}

const INSTALLED: &str = "/Users/someone/.local/share/konnect/bin/konnect";

fn claude_json(command: &str) -> String {
    json!({
        "theme": "dark",
        "mcpServers": {
            "konnect": {"type": "stdio", "command": command, "args": [], "env": {"RUST_LOG": "info"}},
            "other": {"type": "stdio", "command": "/bin/other"}
        }
    })
    .to_string()
}

const CODEX: &str = "model = \"x\"\n\n[mcp_servers.other]\ncommand = \"/bin/other\"\nargs = [\n  \"[not a table]\",\n]\n\n[mcp_servers.konnect]\ncommand = \"/Users/someone/.local/share/konnect/bin/konnect\"\nstartup_timeout_sec = 15\n";

fn switched(text: &str, exe: &str) -> String {
    text.replace(
        &format!("command = \"{INSTALLED}\""),
        &format!("command = \"{exe}\""),
    )
}

#[test]
fn install_records_the_way_back_then_runs_init_and_points_both_clients_at_the_package() {
    let mut fake = Fake::new();
    fake.write(".claude.json", &claude_json(INSTALLED));
    fake.write(".codex/config.toml", CODEX);
    std::fs::set_permissions(
        fake.home().join(".codex/config.toml"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let prefix = fake.package();
    let exe = prefix.join("bin/konnect").to_string_lossy().into_owned();

    activate(&mut fake, &prefix).unwrap();

    let saved = fake.record(&prefix);
    assert_eq!(saved["claude"]["command"], INSTALLED);
    assert_eq!(saved["claude"]["env"]["RUST_LOG"], "info");
    assert_eq!(saved["codex_command"], INSTALLED);
    let mode = std::fs::metadata(rollback_file(&prefix))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "the record may hold an entry's env");

    let calls = fake.calls();
    assert_eq!(calls[0], format!("{exe} init"));
    assert_eq!(calls[1], format!("{exe} init --client codex"));
    assert_eq!(calls[2], "claude mcp remove konnect -s user");
    let added = &fake.calls[3].1;
    assert_eq!(added[..3], ["mcp", "add-json", "konnect"]);
    assert_eq!(added[4..], ["-s", "user"]);
    let entry: Value = serde_json::from_str(&added[3]).unwrap();
    assert_eq!(entry["command"], exe.as_str());
    assert_eq!(
        entry["env"]["RUST_LOG"], "info",
        "the entry's other fields are kept"
    );
    assert_eq!(calls.len(), 4);

    // Codex: only the konnect command line changed, the mode is kept, and the file
    // before is kept.
    let after = fake.read(".codex/config.toml");
    assert_eq!(after, switched(CODEX, &exe));
    assert!(after.contains("command = \"/bin/other\""));
    let mode = std::fs::metadata(fake.home().join(".codex/config.toml"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(fake.read(".codex/config.toml.before-1-kopper"), CODEX);
}

#[test]
fn the_first_original_is_never_overwritten() {
    let mut fake = Fake::new();
    fake.write(".claude.json", &claude_json(INSTALLED));
    fake.write(".codex/config.toml", CODEX);
    let prefix = fake.package();
    activate(&mut fake, &prefix).unwrap();
    let first = std::fs::read_to_string(rollback_file(&prefix)).unwrap();
    let exe = prefix.join("bin/konnect").to_string_lossy().into_owned();

    // Again, with the clients on the package (as `claude mcp add-json` leaves it).
    fake.write(".claude.json", &claude_json(&exe));
    activate(&mut fake, &prefix).unwrap();
    assert_eq!(
        std::fs::read_to_string(rollback_file(&prefix)).unwrap(),
        first
    );
    assert_eq!(
        fake.read(".codex/config.toml.before-1-kopper"),
        CODEX,
        "the backup holds the file before the first switch"
    );

    // Pointed at another binary since: the record still keeps the first original.
    fake.write(".claude.json", &claude_json("/opt/other/konnect"));
    activate(&mut fake, &prefix).unwrap();
    assert_eq!(fake.record(&prefix)["claude"]["command"], INSTALLED);

    // A package that has since been removed is never taken for an original.
    std::fs::remove_file(rollback_file(&prefix)).unwrap();
    let gone = fake.home().join("old/1-kopper/bin/konnect");
    let gone = gone.to_string_lossy();
    fake.write(".claude.json", &claude_json(&gone));
    fake.write(".codex/config.toml", &switched(CODEX, &gone));
    activate(&mut fake, &prefix).unwrap();
    assert!(!kopper_package::present(&rollback_file(&prefix)));
}

#[test]
fn without_entries_claude_gets_one_and_codex_is_left_alone() {
    let mut fake = Fake::new();
    let prefix = fake.package();
    let exe = prefix.join("bin/konnect").to_string_lossy().into_owned();
    let summary = activate(&mut fake, &prefix).unwrap();
    let calls = fake.calls();
    assert_eq!(calls[0], format!("{exe} init"));
    assert!(
        calls[1].starts_with("claude mcp add-json konnect"),
        "{calls:?}"
    );
    assert_eq!(calls.len(), 2, "no remove, no Codex init: {calls:?}");
    assert!(summary.contains("Codex has no konnect entry"));
    assert!(!kopper_package::present(&fake.home().join(".codex")));
    assert!(!kopper_package::present(&rollback_file(&prefix)));
}

#[test]
fn a_failed_add_puts_the_earlier_claude_entry_back() {
    let mut fake = Fake::new();
    fake.write(".claude.json", &claude_json(INSTALLED));
    let prefix = fake.package();
    fake.fail.push(("claude".into(), "add-json".into()));
    let error = activate(&mut fake, &prefix).unwrap_err();
    assert!(
        error.contains("putting the earlier entry back failed too"),
        "{error}"
    );
    assert!(error.contains("--rollback"), "{error}");
    assert!(
        !error.contains("RUST_LOG"),
        "no entry JSON in the message: {error}"
    );
    let adds: Vec<_> = fake
        .calls
        .iter()
        .filter(|(program, args)| program == "claude" && args[1] == "add-json")
        .collect();
    assert_eq!(adds.len(), 2, "the new entry, then the earlier one");
    let back: Value = serde_json::from_str(&adds[1].1[3]).unwrap();
    assert_eq!(back["command"], INSTALLED);
    assert_eq!(fake.record(&prefix)["claude"]["command"], INSTALLED);
}

#[test]
fn a_failed_init_changes_no_entry_and_the_record_is_already_there() {
    let mut fake = Fake::new();
    fake.write(".claude.json", &claude_json(INSTALLED));
    fake.write(".codex/config.toml", CODEX);
    let prefix = fake.package();
    let exe = prefix.join("bin/konnect").to_string_lossy().into_owned();
    fake.fail.push((exe, "init".into()));
    assert!(activate(&mut fake, &prefix).is_err());
    assert!(fake.calls().iter().all(|call| !call.starts_with("claude")));
    assert_eq!(fake.read(".codex/config.toml"), CODEX);
    assert_eq!(fake.record(&prefix)["claude"]["command"], INSTALLED);
}

#[test]
fn entries_this_install_cannot_handle_stop_it_before_any_change() {
    for (claude, why) in [
        ("{ not json", "not JSON"),
        (
            r#"{"mcpServers": {"konnect": {"type": "http", "url": "http://x"}}}"#,
            "not a stdio entry",
        ),
        (
            r#"{"mcpServers": {"konnect": {"type": "stdio"}}}"#,
            "not a stdio entry",
        ),
        (r#"{"mcpServers": {"konnect": "x"}}"#, "not a stdio entry"),
    ] {
        let mut fake = Fake::new();
        fake.write(".claude.json", claude);
        let prefix = fake.package();
        let error = activate(&mut fake, &prefix).unwrap_err();
        assert!(error.contains(why), "{claude}: {error}");
        assert!(fake.calls.is_empty(), "{claude}");
        assert!(!kopper_package::present(&rollback_file(&prefix)));
    }
    for codex in [
        "[mcp_servers.konnect]\ncommand = 'single quoted'\n",
        "[mcp_servers.konnect]\ncommand = \"/x\" # a note\n",
        "[mcp_servers]\nkonnect = { command = \"/x\" }\n",
        "[mcp_servers.konnect.env]\nA = \"b\"\n",
        "[mcp_servers.konnect] # a note\nargs = []\n",
    ] {
        let mut fake = Fake::new();
        fake.write(".codex/config.toml", codex);
        let prefix = fake.package();
        let error = activate(&mut fake, &prefix).unwrap_err();
        assert!(error.contains("nothing was changed"), "{codex}: {error}");
        assert!(fake.calls.is_empty(), "{codex}");
    }
    let mut fake = Fake::new();
    let prefix = fake.package();
    std::fs::write(rollback_file(&prefix), "[1]").unwrap();
    let error = activate(&mut fake, &prefix).unwrap_err();
    assert!(error.contains("not a JSON object"), "{error}");
    assert!(fake.calls.is_empty());
}

#[test]
fn preflight_refuses_what_the_run_would_fail_on() {
    let mut fake = Fake::new();
    let prefix = fake.package();
    let bin = fake.home().join("path-bin");
    std::fs::create_dir_all(&bin).unwrap();
    fake.vars
        .insert("PATH".into(), bin.to_string_lossy().into_owned());
    let error = preflight(&fake, &prefix).unwrap_err();
    assert!(error.contains("`claude` is not on PATH"), "{error}");
    std::fs::write(bin.join("claude"), "").unwrap();
    preflight(&fake, &prefix).unwrap();
    fake.vars
        .insert("CLAUDE_CONFIG_DIR".into(), "/elsewhere".into());
    assert!(preflight(&fake, &prefix)
        .unwrap_err()
        .contains("CLAUDE_CONFIG_DIR"));
    fake.vars.remove("CLAUDE_CONFIG_DIR");
    std::fs::write(rollback_file(&prefix), "not json").unwrap();
    assert!(preflight(&fake, &prefix)
        .unwrap_err()
        .contains("not a JSON object"));
}

#[test]
fn codex_home_names_the_codex_folder() {
    let mut fake = Fake::new();
    fake.write("elsewhere/config.toml", CODEX);
    let elsewhere = fake.home().join("elsewhere").to_string_lossy().into_owned();
    fake.vars.insert("CODEX_HOME".into(), elsewhere);
    let prefix = fake.package();
    let exe = prefix.join("bin/konnect").to_string_lossy().into_owned();
    activate(&mut fake, &prefix).unwrap();
    assert_eq!(fake.read("elsewhere/config.toml"), switched(CODEX, &exe));
}

#[test]
fn the_codex_rewrite_keeps_line_endings_indentation_and_links() {
    let mut fake = Fake::new();
    let crlf = "[mcp_servers.konnect]\r\n  command = \"/Users/someone/.local/share/konnect/bin/konnect\"\r\nx = 1";
    fake.write("dotfiles/codex.toml", crlf);
    std::fs::create_dir_all(fake.home().join(".codex")).unwrap();
    std::os::unix::fs::symlink(
        fake.home().join("dotfiles/codex.toml"),
        fake.home().join(".codex/config.toml"),
    )
    .unwrap();
    let prefix = fake.package();
    let exe = prefix.join("bin/konnect").to_string_lossy().into_owned();
    activate(&mut fake, &prefix).unwrap();
    assert_eq!(
        fake.read("dotfiles/codex.toml"),
        format!("[mcp_servers.konnect]\r\n  command = \"{exe}\"\r\nx = 1")
    );
    let link = std::fs::symlink_metadata(fake.home().join(".codex/config.toml")).unwrap();
    assert!(link.file_type().is_symlink(), "the link is kept");
}

#[test]
fn rollback_puts_every_entry_back_before_any_init() {
    let mut fake = Fake::new();
    fake.write(".claude.json", &claude_json(INSTALLED));
    fake.write(".codex/config.toml", CODEX);
    let prefix = fake.package();
    activate(&mut fake, &prefix).unwrap();
    let exe = prefix.join("bin/konnect").to_string_lossy().into_owned();
    fake.write(".claude.json", &claude_json(&exe));
    fake.calls.clear();

    // The Claude init fails: Codex is still put back, and the record is kept.
    fake.fail.push((INSTALLED.into(), "init".into()));
    let error = rollback(&mut fake, &prefix).unwrap_err();
    assert!(error.contains("Not done"), "{error}");
    assert_eq!(fake.read(".codex/config.toml"), CODEX);
    let calls = fake.calls();
    assert_eq!(calls[0], "claude mcp remove konnect -s user");
    let back: Value = serde_json::from_str(&fake.calls[1].1[3]).unwrap();
    assert_eq!(back["command"], INSTALLED);
    assert_eq!(calls[2], format!("{INSTALLED} init"));
    assert_eq!(calls[3], format!("{INSTALLED} init --client codex"));
    assert!(kopper_package::present(&rollback_file(&prefix)));

    // Again, with the init working: done, and the record is set aside.
    fake.fail.clear();
    fake.write(".claude.json", &claude_json(INSTALLED));
    rollback(&mut fake, &prefix).unwrap();
    assert!(!kopper_package::present(&rollback_file(&prefix)));
    assert!(kopper_package::present(
        &rollback_file(&prefix).with_extension("json.used")
    ));
}

#[test]
fn rollback_runs_no_init_of_a_command_that_is_not_konnect() {
    let mut fake = Fake::new();
    let prefix = fake.package();
    std::fs::write(
        rollback_file(&prefix),
        json!({"claude": {"type": "stdio", "command": "npx", "args": ["x"]}}).to_string(),
    )
    .unwrap();
    let done = rollback(&mut fake, &prefix).unwrap();
    assert!(done.contains("not a konnect binary"), "{done}");
    assert!(fake.calls().iter().all(|call| !call.starts_with("npx")));
}

#[test]
fn rollback_without_a_record_changes_nothing() {
    let mut fake = Fake::new();
    let prefix = fake.package();
    let error = rollback(&mut fake, &prefix).unwrap_err();
    assert!(error.contains("no record"), "{error}");
    assert!(fake.calls.is_empty());
}

#[test]
fn the_codex_reader_finds_only_the_konnect_command() {
    assert_eq!(
        codex_command(CODEX).unwrap(),
        Some((9, INSTALLED.to_string()))
    );
    assert_eq!(
        codex_command("[mcp_servers.other]\ncommand = \"/x\"\n").unwrap(),
        None
    );
    assert_eq!(
        codex_command("\u{feff}[mcp_servers.\"konnect\"]\ncommand = \"C:\\\\a \\\"b\\\"\"\n")
            .unwrap(),
        Some((1, "C:\\a \"b\"".to_string()))
    );
    assert!(codex_command("[mcp_servers.konnect]\ncommand = \"a\\nb\"\n").is_err());
    assert_eq!(toml_quote("C:\\a \"b\""), "\"C:\\\\a \\\"b\\\"\"");
    assert_eq!(strip_comment("x = \"a#b\" # c"), "x = \"a#b\"");
}
