//! Git through the `git` command line: the user's own git, with their config, hooks and
//! credentials, and no second implementation to disagree with it. Pull requests come from the
//! GitHub CLI (`gh`) when it is installed and signed in.
//!
//! Paths in and out are relative to the folder Isoline opened, which may be a subfolder of the
//! work tree: diffs run with `--relative`, and blob reads add the folder's prefix.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use iso_core::ipc::{GitBranch, GitCommit, GitRefs, GitStatus, GitTag, PullRequest, PullRequests};

/// Revision name for uncommitted changes on the head side of a comparison.
pub const WORKTREE: &str = "worktree";

/// An executable on PATH or in the usual install folders. Apps started from the macOS Finder
/// get a minimal PATH (`/usr/bin:/bin:/usr/sbin:/sbin`) that misses Homebrew and Nix.
pub fn find_exe(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    for d in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/run/current-system/sw/bin",
        "/nix/var/nix/profiles/default/bin",
        "/usr/bin",
        "C:\\Program Files\\Git\\cmd",
        "C:\\Program Files\\GitHub CLI",
    ] {
        dirs.push(PathBuf::from(d));
    }
    if let Some(h) = &home {
        dirs.push(h.join(".nix-profile/bin"));
        dirs.push(h.join(".local/bin"));
        dirs.push(h.join("bin"));
    }
    dirs.into_iter().map(|d| d.join(&exe)).find(|p| p.is_file())
}

fn command(exe: &Path, dir: &Path) -> Command {
    let mut c = Command::new(exe);
    c.current_dir(dir)
        // Never wait on a credential prompt nobody can see.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console window flashes up for each call from the GUI app.
        c.creation_flags(0x0800_0000);
    }
    c
}

/// Run to completion with a time limit; stdout on success, git's own message on failure.
fn run_cmd(mut c: Command, timeout: Duration) -> Result<Vec<u8>> {
    let mut child = c.spawn().context("could not start the command")?;
    let mut out = child.stdout.take().expect("piped");
    let mut err = child.stderr.take().expect("piped");
    let t_out = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = out.read_to_end(&mut b);
        b
    });
    let t_err = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = err.read_to_end(&mut b);
        b
    });
    let t0 = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if t0.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("timed out after {} s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let stdout = t_out.join().unwrap_or_default();
    let stderr = t_err.join().unwrap_or_default();
    if !status.success() {
        let msg = String::from_utf8_lossy(&stderr).trim().to_string();
        let msg = msg
            .lines()
            .map(|l| {
                l.trim_start_matches("error: ")
                    .trim_start_matches("fatal: ")
            })
            .collect::<Vec<_>>()
            .join("\n");
        bail!(if msg.is_empty() {
            format!("exited with {status}")
        } else {
            msg
        });
    }
    Ok(stdout)
}

/// One change between two revisions, from `git diff --name-status` and `--numstat`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawChange {
    pub path: String,
    pub old_path: Option<String>,
    /// `A`, `M`, `D`, `R`, `C`, `T` (type change).
    pub status: char,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
}

/// A git work tree, seen from the folder Isoline opened.
#[derive(Clone, Debug)]
pub struct Repo {
    git: PathBuf,
    /// The folder Isoline opened.
    pub dir: PathBuf,
    /// Top of the work tree.
    pub top: PathBuf,
    /// `dir` relative to `top`, with a trailing `/`; empty at the top.
    pub prefix: String,
    git_dir: PathBuf,
}

impl Repo {
    /// `None` when git is missing or `dir` is not inside a work tree.
    pub fn open(dir: &Path) -> Option<Repo> {
        let git = find_exe("git")?;
        let out = run_cmd(
            {
                let mut c = command(&git, dir);
                c.args([
                    "rev-parse",
                    "--is-inside-work-tree",
                    "--show-toplevel",
                    "--show-prefix",
                    "--absolute-git-dir",
                ]);
                c
            },
            Duration::from_secs(10),
        )
        .ok()?;
        let s = String::from_utf8_lossy(&out);
        let mut l = s.lines();
        if l.next()? != "true" {
            return None;
        }
        let top = PathBuf::from(l.next()?);
        let prefix = l.next().unwrap_or("").to_string();
        let git_dir = PathBuf::from(l.next()?);
        Some(Repo {
            git,
            dir: dir.to_path_buf(),
            top,
            prefix,
            git_dir,
        })
    }

    fn git(&self, args: &[&str]) -> Result<Vec<u8>> {
        self.git_t(args, Duration::from_secs(60))
    }

    fn git_t(&self, args: &[&str], timeout: Duration) -> Result<Vec<u8>> {
        let mut c = command(&self.git, &self.dir);
        c.args(args);
        run_cmd(c, timeout).with_context(|| format!("git {}", args.first().unwrap_or(&"")))
    }

    fn git_str(&self, args: &[&str]) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.git(args)?).trim().to_string())
    }

    /// Branch, upstream, ahead/behind and counts of changed files.
    pub fn status(&self) -> Result<GitStatus> {
        // GIT_OPTIONAL_LOCKS=0: a status poll must never take index.lock from the user's git.
        let mut c = command(&self.git, &self.dir);
        c.env("GIT_OPTIONAL_LOCKS", "0").args([
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=normal",
        ]);
        let out = run_cmd(c, Duration::from_secs(30)).context("git status")?;
        let mut st = parse_status(&out);
        st.operation = self.operation();
        st.github = self
            .git_str(&["remote", "-v"])
            .map(|r| r.contains("github.com"))
            .unwrap_or(false);
        Ok(st)
    }

    /// Size and modification time of the files git rewrites when HEAD, the index, refs or
    /// remote state change. Equal stamps mean `status` would say the same, unless work-tree
    /// files changed (the caller watches those).
    pub fn state_stamp(&self) -> Vec<(u64, u128)> {
        let mut v: Vec<(u64, u128)> = [
            "HEAD",
            "index",
            "FETCH_HEAD",
            "ORIG_HEAD",
            "packed-refs",
            "logs/HEAD",
            "MERGE_HEAD",
            "rebase-merge",
            "rebase-apply",
        ]
        .iter()
        .map(|f| {
            std::fs::metadata(self.git_dir.join(f))
                .map(|m| {
                    let t = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos())
                        .unwrap_or(0);
                    (m.len(), t)
                })
                .unwrap_or((0, 0))
        })
        .collect();
        // The current branch's ref file, which a commit rewrites.
        if let Ok(head) = std::fs::read_to_string(self.git_dir.join("HEAD"))
            && let Some(r) = head.trim().strip_prefix("ref: ")
            && let Ok(m) = std::fs::metadata(self.git_dir.join(r))
        {
            let t = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            v.push((m.len(), t));
        }
        v
    }

    fn operation(&self) -> Option<String> {
        let g = &self.git_dir;
        let op = if g.join("rebase-merge").exists() || g.join("rebase-apply").exists() {
            "rebase"
        } else if g.join("MERGE_HEAD").exists() {
            "merge"
        } else if g.join("CHERRY_PICK_HEAD").exists() {
            "cherry-pick"
        } else if g.join("REVERT_HEAD").exists() {
            "revert"
        } else if g.join("BISECT_LOG").exists() {
            "bisect"
        } else {
            return None;
        };
        Some(op.to_string())
    }

    /// Branches (local, then remote), tags and recent commits on HEAD.
    pub fn refs(&self) -> Result<GitRefs> {
        let fmt = "%(refname)%00%(objectname:short)%00%(subject)%00%(authorname)%00%(committerdate:unix)%00%(upstream:short)%00%(HEAD)%00%(symref)";
        let out = self.git(&[
            "for-each-ref",
            "--sort=-committerdate",
            &format!("--format={fmt}"),
            "refs/heads",
            "refs/remotes",
        ])?;
        let mut local = Vec::new();
        let mut remote = Vec::new();
        for line in String::from_utf8_lossy(&out).lines() {
            let f: Vec<&str> = line.split('\0').collect();
            if f.len() < 8 || !f[7].is_empty() {
                continue; // refs/remotes/origin/HEAD and other symbolic refs
            }
            let (name, is_remote) = if let Some(n) = f[0].strip_prefix("refs/heads/") {
                (n, false)
            } else if let Some(n) = f[0].strip_prefix("refs/remotes/") {
                (n, true)
            } else {
                continue;
            };
            let b = GitBranch {
                name: name.to_string(),
                remote: is_remote,
                current: f[6] == "*",
                sha: f[1].to_string(),
                subject: f[2].to_string(),
                author: f[3].to_string(),
                date: f[4].parse().unwrap_or(0),
                upstream: (!f[5].is_empty()).then(|| f[5].to_string()),
            };
            if is_remote { &mut remote } else { &mut local }.push(b);
        }
        local.extend(remote);
        let tags_out = self.git(&[
            "for-each-ref",
            "--sort=-creatordate",
            "--count=200",
            "--format=%(refname:short)%00%(if)%(*objectname)%(then)%(*objectname)%(else)%(objectname)%(end)%00%(creatordate:unix)",
            "refs/tags",
        ])?;
        let tags = String::from_utf8_lossy(&tags_out)
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split('\0').collect();
                (f.len() >= 3).then(|| GitTag {
                    name: f[0].to_string(),
                    sha: f[1].to_string(),
                    date: f[2].parse().unwrap_or(0),
                })
            })
            .collect();
        let commits = self.log(&["-n", "100", "HEAD"]).unwrap_or_default();
        Ok(GitRefs {
            branches: local,
            tags,
            commits,
        })
    }

    /// `git log` with extra arguments, parsed.
    pub fn log(&self, args: &[&str]) -> Result<Vec<GitCommit>> {
        let mut a = vec!["log", "--format=%H%x00%h%x00%s%x00%an%x00%ct%x00%D%x1e"];
        a.extend_from_slice(args);
        a.push("--");
        let out = self.git(&a)?;
        Ok(String::from_utf8_lossy(&out)
            .split('\x1e')
            .filter_map(|rec| {
                let rec = rec.trim_start_matches('\n');
                let f: Vec<&str> = rec.split('\0').collect();
                (f.len() >= 6).then(|| GitCommit {
                    sha: f[0].to_string(),
                    short: f[1].to_string(),
                    subject: f[2].to_string(),
                    author: f[3].to_string(),
                    date: f[4].parse().unwrap_or(0),
                    refs: f[5]
                        .split(", ")
                        .map(|r| {
                            r.trim()
                                .trim_start_matches("HEAD -> ")
                                .trim_start_matches("tag: ")
                                .to_string()
                        })
                        .filter(|r| !r.is_empty() && r != "HEAD" && !r.ends_with("/HEAD"))
                        .collect(),
                })
            })
            .collect())
    }

    /// Switch branches. A remote branch (`origin/feature`) switches to the local branch of the
    /// same name, creating it to track the remote one when it does not exist yet.
    pub fn switch(&self, name: &str, create: bool) -> Result<()> {
        anyhow::ensure!(
            !name.is_empty() && !name.starts_with('-'),
            "not a branch name: {name}"
        );
        if create {
            self.git(&["switch", "-c", name])?;
            return Ok(());
        }
        let is_local = self
            .git(&[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{name}"),
            ])
            .is_ok();
        if !is_local
            && self
                .git(&[
                    "show-ref",
                    "--verify",
                    "--quiet",
                    &format!("refs/remotes/{name}"),
                ])
                .is_ok()
        {
            let local = name.split_once('/').map(|(_, b)| b).unwrap_or(name);
            let has_local = self
                .git(&[
                    "show-ref",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{local}"),
                ])
                .is_ok();
            if has_local {
                self.git(&["switch", local])?;
            } else {
                self.git(&["switch", "--track", name])?;
            }
            return Ok(());
        }
        self.git(&["switch", name])?;
        Ok(())
    }

    /// Fetch every remote.
    pub fn fetch(&self) -> Result<()> {
        self.git_t(&["fetch", "--all", "--quiet"], Duration::from_secs(120))?;
        Ok(())
    }

    /// Full commit id of a revision.
    pub fn rev_parse(&self, rev: &str) -> Result<String> {
        anyhow::ensure!(!rev.starts_with('-'), "not a revision: {rev}");
        self.git_str(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ])
        .map_err(|_| anyhow!("no such branch, tag or commit: {rev}"))
    }

    pub fn has_commit(&self, sha: &str) -> bool {
        self.git(&["cat-file", "-e", &format!("{sha}^{{commit}}")])
            .is_ok()
    }

    pub fn merge_base(&self, a: &str, b: &str) -> Result<String> {
        self.git_str(&["merge-base", a, b])
            .map_err(|_| anyhow!("{a} and {b} share no history"))
    }

    pub fn subject(&self, sha: &str) -> Option<String> {
        self.git_str(&["log", "-1", "--format=%s", sha, "--"]).ok()
    }

    /// Changes from `base` to `head` (`None`: the working tree, untracked files included),
    /// limited to the opened folder, with renames detected.
    pub fn diff(&self, base: &str, head: Option<&str>) -> Result<Vec<RawChange>> {
        let mut revs = vec![base];
        if let Some(h) = head {
            revs.push(h);
        }
        let mut ns = vec![
            "diff",
            "-z",
            "--name-status",
            "-M",
            "--relative",
            "--no-ext-diff",
        ];
        ns.extend(&revs);
        ns.push("--");
        let mut nm = vec![
            "diff",
            "-z",
            "--numstat",
            "-M",
            "--relative",
            "--no-ext-diff",
        ];
        nm.extend(&revs);
        nm.push("--");
        let mut changes = parse_name_status(&self.git(&ns).map_err(missing_objects)?);
        let counts = parse_numstat(&self.git(&nm)?);
        for c in &mut changes {
            if let Some(&(a, d, bin)) = counts.get(&c.path) {
                c.additions = a;
                c.deletions = d;
                c.binary = bin;
            }
        }
        if head.is_none() {
            let out = self.git(&["ls-files", "-z", "--others", "--exclude-standard"])?;
            for p in out.split(|&b| b == 0).filter(|p| !p.is_empty()) {
                let path = String::from_utf8_lossy(p).into_owned();
                let (lines, binary) = std::fs::read(self.dir.join(&path))
                    .map(|b| {
                        let bin = b.iter().take(8000).any(|&x| x == 0);
                        (if bin { 0 } else { count_lines(&b) }, bin)
                    })
                    .unwrap_or((0, false));
                changes.push(RawChange {
                    path,
                    old_path: None,
                    status: 'A',
                    additions: lines,
                    deletions: 0,
                    binary,
                });
            }
        }
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(changes)
    }

    /// Every file at a revision (`None`: tracked and untracked files in the working tree),
    /// relative to the opened folder.
    pub fn files_at(&self, rev: Option<&str>) -> Result<Vec<String>> {
        let out = match rev {
            Some(r) => self.git(&["ls-tree", "-r", "-z", "--name-only", r])?,
            None => self.git(&[
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
            ])?,
        };
        Ok(out
            .split(|&b| b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect())
    }

    /// File contents at a revision, read in one `git cat-file --batch`. Missing files are
    /// absent from the map. Paths are relative to the opened folder.
    pub fn read_blobs(&self, rev: &str, paths: &[String]) -> Result<HashMap<String, Vec<u8>>> {
        let mut out = HashMap::new();
        if paths.is_empty() {
            return Ok(out);
        }
        let mut c = command(&self.git, &self.dir);
        c.args(["cat-file", "--batch"]).stdin(Stdio::piped());
        let mut child = c.spawn().context("git cat-file")?;
        let mut stdin = child.stdin.take().expect("piped");
        let reqs: Vec<String> = paths
            .iter()
            .map(|p| format!("{rev}:{}{p}\n", self.prefix))
            .collect();
        let writer = std::thread::spawn(move || {
            for r in reqs {
                if stdin.write_all(r.as_bytes()).is_err() {
                    break;
                }
            }
        });
        let mut rd = BufReader::new(child.stdout.take().expect("piped"));
        for p in paths {
            let mut header = String::new();
            if rd.read_line(&mut header)? == 0 {
                break;
            }
            let h: Vec<&str> = header.split_whitespace().collect();
            // "<oid> blob <size>" or "<spec> missing" / "ambiguous".
            if h.len() == 3 && h[1] == "blob" {
                let size: usize = h[2].parse().unwrap_or(0);
                let mut buf = vec![0u8; size + 1]; // content plus the trailing newline
                rd.read_exact(&mut buf)?;
                buf.pop();
                out.insert(p.clone(), buf);
            } else if h.len() == 3 {
                // A tree or submodule commit: skip its body.
                let size: usize = h[2].parse().unwrap_or(0);
                let mut buf = vec![0u8; size + 1];
                rd.read_exact(&mut buf)?;
            }
        }
        let _ = writer.join();
        let _ = child.wait();
        Ok(out)
    }

    /// The GitHub CLI; `ISOLINE_GH` names another executable (tests use a stand-in).
    fn gh(&self) -> Option<PathBuf> {
        match std::env::var_os("ISOLINE_GH") {
            Some(p) => Some(PathBuf::from(p)),
            None => find_exe("gh"),
        }
    }

    /// Open pull requests through `gh`, or why they cannot be listed.
    pub fn pull_requests(&self) -> PullRequests {
        let Some(gh) = self.gh() else {
            return PullRequests {
                available: false,
                message: Some(
                    "Install the GitHub CLI (gh) and run `gh auth login` to list pull requests."
                        .into(),
                ),
                items: Vec::new(),
            };
        };
        let mut c = command(&gh, &self.dir);
        c.args([
            "pr", "list", "--state", "open", "--limit", "100", "--json", PR_FIELDS,
        ]);
        match run_cmd(c, Duration::from_secs(30)) {
            Ok(out) => match serde_json::from_slice::<Vec<serde_json::Value>>(&out) {
                Ok(v) => PullRequests {
                    available: true,
                    message: None,
                    items: v.iter().map(pr_from_json).collect(),
                },
                Err(e) => PullRequests {
                    available: false,
                    message: Some(format!("Could not read gh's answer: {e}")),
                    items: Vec::new(),
                },
            },
            Err(e) => PullRequests {
                available: false,
                message: Some(gh_message(&format!("{e:#}"))),
                items: Vec::new(),
            },
        }
    }

    /// A pull request and the commits of its base branch tip and head, fetched from the remote
    /// when they are not here yet.
    pub fn pull_request(&self, number: u32) -> Result<(PullRequest, String, String)> {
        let gh = self
            .gh()
            .ok_or_else(|| anyhow!("Install the GitHub CLI (gh) to compare pull requests."))?;
        let mut c = command(&gh, &self.dir);
        c.args([
            "pr",
            "view",
            &number.to_string(),
            "--json",
            &format!("{PR_FIELDS},headRefOid,baseRefOid"),
        ]);
        let out = run_cmd(c, Duration::from_secs(30))
            .map_err(|e| anyhow!(gh_message(&format!("{e:#}"))))?;
        let v: serde_json::Value = serde_json::from_slice(&out)?;
        let pr = pr_from_json(&v);
        let head = v["headRefOid"].as_str().unwrap_or_default().to_string();
        let base = v["baseRefOid"].as_str().unwrap_or_default().to_string();
        if head.is_empty() || !self.has_commit(&head) || base.is_empty() || !self.has_commit(&base)
        {
            let remote = self.github_remote().unwrap_or_else(|| "origin".into());
            self.git_t(
                &[
                    "fetch",
                    "--no-tags",
                    "--quiet",
                    &remote,
                    &format!("refs/pull/{number}/head"),
                    &format!("refs/heads/{}", pr.base_ref),
                ],
                Duration::from_secs(120),
            )
            .with_context(|| format!("could not fetch pull request #{number} from {remote}"))?;
        }
        let head = if head.is_empty() || !self.has_commit(&head) {
            self.fetch_head_line(0)?
        } else {
            head
        };
        let base = if base.is_empty() || !self.has_commit(&base) {
            self.fetch_head_line(1)?
        } else {
            base
        };
        Ok((pr, base, head))
    }

    fn fetch_head_line(&self, i: usize) -> Result<String> {
        let s = std::fs::read_to_string(self.git_dir.join("FETCH_HEAD"))?;
        s.lines()
            .nth(i)
            .and_then(|l| l.split_whitespace().next())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("the fetch returned no commit"))
    }

    /// The remote that points at GitHub (`origin` when several do).
    fn github_remote(&self) -> Option<String> {
        let out = self.git_str(&["remote", "-v"]).ok()?;
        let mut found: Vec<&str> = out
            .lines()
            .filter(|l| l.contains("github.com"))
            .filter_map(|l| l.split_whitespace().next())
            .collect();
        found.dedup();
        if found.contains(&"origin") {
            return Some("origin".into());
        }
        found.first().map(|s| s.to_string())
    }
}

/// A partial or shallow clone that lacks a revision's objects says so in git's terms.
fn missing_objects(e: anyhow::Error) -> anyhow::Error {
    let m = format!("{e:#}");
    if m.contains("unable to read") || m.contains("bad object") || m.contains("missing blob") {
        anyhow!(
            "{m}\nThis clone does not have that revision's files (a shallow or partial clone). Fetch the history, or compare more recent revisions."
        )
    } else {
        e
    }
}

const PR_FIELDS: &str = "number,title,author,headRefName,baseRefName,isDraft,url,updatedAt,additions,deletions,changedFiles";

fn gh_message(err: &str) -> String {
    let e = err.to_lowercase();
    if e.contains("gh auth login") || e.contains("not logged") || e.contains("authentication") {
        "Run `gh auth login` in a terminal to list pull requests.".into()
    } else if e.contains("no git remotes") || e.contains("none of the git remotes") {
        "This repo has no GitHub remote, so it has no pull requests to list.".into()
    } else {
        format!("gh: {}", err.lines().last().unwrap_or(err))
    }
}

fn pr_from_json(v: &serde_json::Value) -> PullRequest {
    let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
    let n = |k: &str| v[k].as_u64().unwrap_or(0) as u32;
    PullRequest {
        number: n("number"),
        title: s("title"),
        author: v["author"]["login"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        head_ref: s("headRefName"),
        base_ref: s("baseRefName"),
        draft: v["isDraft"].as_bool().unwrap_or(false),
        url: s("url"),
        updated: parse_iso8601(v["updatedAt"].as_str().unwrap_or_default()).unwrap_or(0),
        additions: n("additions"),
        deletions: n("deletions"),
        changed_files: n("changedFiles"),
    }
}

/// `2026-11-17T10:04:05Z` to unix seconds (the only form GitHub sends).
pub fn parse_iso8601(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    // Days from civil (Howard Hinnant).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

fn count_lines(b: &[u8]) -> u32 {
    if b.is_empty() {
        return 0;
    }
    let n = b.iter().filter(|&&x| x == b'\n').count() as u32;
    if b.last() == Some(&b'\n') { n } else { n + 1 }
}

/// `git status --porcelain=v2 --branch -z`.
pub fn parse_status(out: &[u8]) -> GitStatus {
    let mut st = GitStatus {
        is_repo: true,
        branch: None,
        head: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        changed: 0,
        untracked: 0,
        operation: None,
        github: false,
    };
    let mut fields = out.split(|&b| b == 0);
    while let Some(f) = fields.next() {
        let line = String::from_utf8_lossy(f);
        if let Some(h) = line.strip_prefix("# branch.oid ") {
            if h != "(initial)" {
                st.head = Some(h.chars().take(7).collect());
            }
        } else if let Some(h) = line.strip_prefix("# branch.head ") {
            if h != "(detached)" {
                st.branch = Some(h.to_string());
            }
        } else if let Some(u) = line.strip_prefix("# branch.upstream ") {
            st.upstream = Some(u.to_string());
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            let mut it = ab.split_whitespace();
            st.ahead = it
                .next()
                .and_then(|a| a.trim_start_matches('+').parse().ok())
                .unwrap_or(0);
            st.behind = it
                .next()
                .and_then(|b| b.trim_start_matches('-').parse().ok())
                .unwrap_or(0);
        } else if line.starts_with("1 ") || line.starts_with("u ") {
            st.changed += 1;
        } else if line.starts_with("2 ") {
            st.changed += 1;
            fields.next(); // the original path of a rename
        } else if line.starts_with("? ") {
            st.untracked += 1;
        }
    }
    st
}

/// `git diff -z --name-status -M`: `M\0path\0`, `R100\0old\0new\0`.
pub fn parse_name_status(out: &[u8]) -> Vec<RawChange> {
    let mut v = Vec::new();
    let mut it = out.split(|&b| b == 0).filter(|f| !f.is_empty());
    while let Some(code) = it.next() {
        let status = code.first().copied().unwrap_or(b'M') as char;
        let s = |f: &[u8]| String::from_utf8_lossy(f).into_owned();
        if status == 'R' || status == 'C' {
            let (Some(old), Some(new)) = (it.next(), it.next()) else {
                break;
            };
            v.push(RawChange {
                path: s(new),
                old_path: Some(s(old)),
                status: if status == 'C' { 'A' } else { 'R' },
                additions: 0,
                deletions: 0,
                binary: false,
            });
        } else {
            let Some(p) = it.next() else { break };
            v.push(RawChange {
                path: s(p),
                old_path: None,
                status: if status == 'T' { 'M' } else { status },
                additions: 0,
                deletions: 0,
                binary: false,
            });
        }
    }
    v
}

/// `git diff -z --numstat -M`: `a\td\tpath\0`, renames `a\td\t\0old\0new\0`; `-` for binary.
pub fn parse_numstat(out: &[u8]) -> HashMap<String, (u32, u32, bool)> {
    let mut m = HashMap::new();
    let mut it = out.split(|&b| b == 0);
    while let Some(f) = it.next() {
        if f.is_empty() {
            continue;
        }
        let line = String::from_utf8_lossy(f);
        let mut parts = line.splitn(3, '\t');
        let (Some(a), Some(d), Some(p)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let binary = a == "-";
        let path = if p.is_empty() {
            let _old = it.next();
            match it.next() {
                Some(n) => String::from_utf8_lossy(n).into_owned(),
                None => break,
            }
        } else {
            p.to_string()
        };
        m.insert(
            path,
            (a.parse().unwrap_or(0), d.parse().unwrap_or(0), binary),
        );
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_v2() {
        let out = b"# branch.oid 0123456789abcdef\0# branch.head feature/x\0# branch.upstream origin/feature/x\0# branch.ab +2 -1\x001 .M N... 100644 100644 100644 aa bb src/a.ts\x002 R. N... 100644 100644 100644 aa bb R100 src/new.ts\0src/old.ts\0? notes.md\0? tmp/x\0";
        let s = parse_status(out);
        assert_eq!(s.branch.as_deref(), Some("feature/x"));
        assert_eq!(s.head.as_deref(), Some("0123456"));
        assert_eq!(s.upstream.as_deref(), Some("origin/feature/x"));
        assert_eq!((s.ahead, s.behind, s.changed, s.untracked), (2, 1, 2, 2));
        let d = parse_status(b"# branch.oid (initial)\0# branch.head (detached)\0");
        assert_eq!((d.branch, d.head), (None, None));
    }

    #[test]
    fn name_status_and_numstat() {
        let ns = parse_name_status(b"M\0a.ts\0R087\0old.ts\0new.ts\0A\0b.ts\0D\0c.ts\0");
        assert_eq!(ns.len(), 4);
        assert_eq!(ns[1].status, 'R');
        assert_eq!(ns[1].old_path.as_deref(), Some("old.ts"));
        assert_eq!(ns[1].path, "new.ts");
        let n = parse_numstat(b"3\t1\ta.ts\x00002\t2\t\0old.ts\0new.ts\0-\t-\timg.png\0");
        assert_eq!(n["a.ts"], (3, 1, false));
        assert_eq!(n["new.ts"], (2, 2, false));
        assert_eq!(n["img.png"], (0, 0, true));
    }

    #[test]
    fn iso_dates() {
        assert_eq!(parse_iso8601("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso8601("2026-11-17T10:04:05Z"), Some(1_794_909_845));
    }

    fn sh(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .current_dir(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .expect("git");
        assert!(
            ok.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&ok.stderr)
        );
    }

    #[test]
    fn a_real_repo() {
        if find_exe("git").is_none() {
            return;
        }
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        sh(d, &["init", "-q", "-b", "main"]);
        std::fs::create_dir_all(d.join("app/src")).unwrap();
        std::fs::write(d.join("app/src/a.ts"), "export function a() {}\n").unwrap();
        std::fs::write(d.join("app/src/b.ts"), "export const b = 1;\n").unwrap();
        std::fs::write(d.join("top.txt"), "outside the opened folder\n").unwrap();
        sh(d, &["add", "."]);
        sh(d, &["commit", "-q", "-m", "first"]);
        sh(d, &["switch", "-q", "-c", "feature"]);
        std::fs::write(
            d.join("app/src/a.ts"),
            "export function a() {\n  return 1;\n}\n",
        )
        .unwrap();
        std::fs::remove_file(d.join("app/src/b.ts")).unwrap();
        std::fs::write(d.join("top.txt"), "changed\n").unwrap();
        sh(d, &["commit", "-q", "-am", "second"]);
        std::fs::write(d.join("app/src/new.ts"), "export {};\n").unwrap();

        // Opened at a subfolder: paths are relative to it, and the change above it is not seen.
        let r = Repo::open(&d.join("app")).expect("a work tree");
        assert_eq!(r.prefix, "app/");
        let st = r.status().unwrap();
        assert_eq!(st.branch.as_deref(), Some("feature"));
        assert_eq!(st.untracked, 1);
        let refs = r.refs().unwrap();
        let names: Vec<&str> = refs.branches.iter().map(|b| b.name.as_str()).collect();
        assert!(names.contains(&"main") && names.contains(&"feature"));
        assert!(
            refs.branches
                .iter()
                .any(|b| b.current && b.name == "feature")
        );
        assert_eq!(refs.commits.len(), 2);
        assert_eq!(refs.commits[0].subject, "second");

        let d1 = r.diff("main", Some("feature")).unwrap();
        let paths: Vec<(&str, char)> = d1.iter().map(|c| (c.path.as_str(), c.status)).collect();
        assert_eq!(paths, vec![("src/a.ts", 'M'), ("src/b.ts", 'D')]);
        assert_eq!((d1[0].additions, d1[0].deletions), (3, 1));
        let wt = r.diff("feature", None).unwrap();
        assert_eq!(wt.len(), 1);
        assert_eq!((wt[0].path.as_str(), wt[0].status), ("src/new.ts", 'A'));

        let blobs = r
            .read_blobs(
                "main",
                &["src/a.ts".into(), "src/b.ts".into(), "src/none.ts".into()],
            )
            .unwrap();
        assert_eq!(blobs["src/a.ts"], b"export function a() {}\n");
        assert!(blobs.contains_key("src/b.ts") && !blobs.contains_key("src/none.ts"));
        let files = r.files_at(Some("main")).unwrap();
        assert_eq!(files, vec!["src/a.ts".to_string(), "src/b.ts".to_string()]);

        assert!(r.rev_parse("nope").is_err());
        let mb = r.merge_base("main", "feature").unwrap();
        assert_eq!(mb, r.rev_parse("main").unwrap());

        std::fs::remove_file(d.join("app/src/new.ts")).unwrap();
        r.switch("main", false).unwrap();
        assert_eq!(r.status().unwrap().branch.as_deref(), Some("main"));
        r.switch("topic", true).unwrap();
        assert_eq!(r.status().unwrap().branch.as_deref(), Some("topic"));
        let e = r.switch("does-not-exist", false).unwrap_err();
        assert!(format!("{e:#}").contains("does-not-exist"), "{e:#}");
    }

    /// A pull request whose head is only on the remote: fetched through refs/pull/N/head.
    /// `gh` is a stand-in script that answers like the real one.
    #[cfg(unix)]
    #[test]
    fn pull_request_is_fetched_when_missing() {
        if find_exe("git").is_none() {
            return;
        }
        let t = tempfile::tempdir().unwrap();
        let up = t.path().join("up");
        std::fs::create_dir_all(&up).unwrap();
        sh(&up, &["init", "-q", "-b", "main"]);
        std::fs::write(up.join("a.ts"), "export const a = 1;\n").unwrap();
        sh(&up, &["add", "."]);
        sh(&up, &["commit", "-q", "-m", "base"]);
        let remote = t.path().join("remote.git");
        sh(t.path(), &["clone", "-q", "--bare", "up", "remote.git"]);
        sh(&up, &["switch", "-q", "-c", "fix"]);
        std::fs::write(up.join("a.ts"), "export const a = 2;\n").unwrap();
        sh(&up, &["commit", "-q", "-am", "fix a"]);
        let remote_s = remote.to_string_lossy().into_owned();
        sh(&up, &["push", "-q", &remote_s, "fix:refs/pull/7/head"]);
        let head = String::from_utf8(
            Command::new("git")
                .current_dir(&up)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();
        let base = String::from_utf8(
            Command::new("git")
                .current_dir(&up)
                .args(["rev-parse", "main"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();
        sh(t.path(), &["clone", "-q", "--no-local", &remote_s, "work"]);
        let work = t.path().join("work");
        let gh = t.path().join("gh");
        let json = format!(
            r#"{{"number":7,"title":"Fix a","author":{{"login":"someone"}},"headRefName":"fix","baseRefName":"main","isDraft":false,"url":"https://github.com/o/r/pull/7","updatedAt":"2026-11-17T10:04:05Z","additions":1,"deletions":1,"changedFiles":1,"headRefOid":"{head}","baseRefOid":"{base}"}}"#
        );
        std::fs::write(
            &gh,
            format!(
                "#!/bin/sh\nif [ \"$2\" = list ]; then echo '[{json}]'; else echo '{json}'; fi\n"
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        // SAFETY: this test alone reads ISOLINE_GH, and sets it before any thread that might.
        unsafe { std::env::set_var("ISOLINE_GH", &gh) };

        let r = Repo::open(&work).unwrap();
        assert!(
            !r.has_commit(&head),
            "the PR head is not here before the fetch"
        );
        let list = r.pull_requests();
        assert!(list.available, "{:?}", list.message);
        assert_eq!(list.items[0].number, 7);
        assert_eq!(list.items[0].author, "someone");
        assert_eq!(list.items[0].updated, 1_794_909_845);
        let (pr, b, h) = r.pull_request(7).unwrap();
        assert_eq!(
            (pr.head_ref.as_str(), pr.base_ref.as_str()),
            ("fix", "main")
        );
        assert_eq!((b, h.clone()), (base, head.clone()));
        assert!(r.has_commit(&head));
        let d = r
            .diff(&r.merge_base("main", &h).unwrap(), Some(&h))
            .unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].path, "a.ts");
    }
}
