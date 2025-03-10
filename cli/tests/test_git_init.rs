// Copyright 2024 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;

use indoc::formatdoc;
use test_case::test_case;

use crate::common::get_stderr_string;
use crate::common::get_stdout_string;
use crate::common::strip_last_line;
use crate::common::to_toml_value;
use crate::common::TestEnvironment;

fn init_git_repo(git_repo_path: &Path, bare: bool) -> git2::Repository {
    init_git_repo_with_opts(git_repo_path, git2::RepositoryInitOptions::new().bare(bare))
}

fn init_git_repo_with_opts(
    git_repo_path: &Path,
    opts: &git2::RepositoryInitOptions,
) -> git2::Repository {
    let git_repo = git2::Repository::init_opts(git_repo_path, opts).unwrap();
    let git_blob_oid = git_repo.blob(b"some content").unwrap();
    let mut git_tree_builder = git_repo.treebuilder(None).unwrap();
    git_tree_builder
        .insert("some-file", git_blob_oid, 0o100644)
        .unwrap();
    let git_tree_id = git_tree_builder.write().unwrap();
    drop(git_tree_builder);
    let git_tree = git_repo.find_tree(git_tree_id).unwrap();
    let git_signature = git2::Signature::new(
        "Git User",
        "git.user@example.com",
        &git2::Time::new(123, 60),
    )
    .unwrap();
    git_repo
        .commit(
            Some("refs/heads/my-bookmark"),
            &git_signature,
            &git_signature,
            "My commit message",
            &git_tree,
            &[],
        )
        .unwrap();
    drop(git_tree);
    git_repo.set_head("refs/heads/my-bookmark").unwrap();
    git_repo
}

fn get_bookmark_output(test_env: &TestEnvironment, repo_path: &Path) -> String {
    test_env.jj_cmd_success(repo_path, &["bookmark", "list", "--all-remotes"])
}

fn get_log_output(test_env: &TestEnvironment, workspace_root: &Path) -> String {
    let template = r#"
    separate(" ",
      commit_id.short(),
      bookmarks,
      if(git_head, "git_head()"),
      description,
    )"#;
    test_env.jj_cmd_success(workspace_root, &["log", "-T", template, "-r=all()"])
}

fn read_git_target(workspace_root: &Path) -> String {
    let mut path = workspace_root.to_path_buf();
    path.extend([".jj", "repo", "store", "git_target"]);
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn test_git_init_internal() {
    let test_env = TestEnvironment::default();
    let (stdout, stderr) = test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Initialized repo in "repo"
    "###);

    let workspace_root = test_env.env_root().join("repo");
    let jj_path = workspace_root.join(".jj");
    let repo_path = jj_path.join("repo");
    let store_path = repo_path.join("store");
    assert!(workspace_root.is_dir());
    assert!(jj_path.is_dir());
    assert!(jj_path.join("working_copy").is_dir());
    assert!(repo_path.is_dir());
    assert!(store_path.is_dir());
    assert!(store_path.join("git").is_dir());
    assert_eq!(read_git_target(&workspace_root), "git");
}

#[test]
fn test_git_init_internal_ignore_working_copy() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    std::fs::create_dir(&workspace_root).unwrap();
    std::fs::write(workspace_root.join("file1"), "").unwrap();

    let stderr =
        test_env.jj_cmd_cli_error(&workspace_root, &["git", "init", "--ignore-working-copy"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: --ignore-working-copy is not respected
    "###);
}

#[test]
fn test_git_init_internal_at_operation() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    std::fs::create_dir(&workspace_root).unwrap();

    let stderr = test_env.jj_cmd_cli_error(&workspace_root, &["git", "init", "--at-op=@-"]);
    insta::assert_snapshot!(stderr, @r###"
    Error: --at-op is not respected
    "###);
}

#[test_case(false; "full")]
#[test_case(true; "bare")]
fn test_git_init_external(bare: bool) {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    init_git_repo(&git_repo_path, bare);

    let (stdout, stderr) = test_env.jj_cmd_ok(
        test_env.env_root(),
        &[
            "git",
            "init",
            "repo",
            "--git-repo",
            git_repo_path.to_str().unwrap(),
        ],
    );
    insta::allow_duplicates! {
        insta::assert_snapshot!(stdout, @"");
        insta::assert_snapshot!(stderr, @r###"
        Done importing changes from the underlying Git repo.
        Working copy now at: sqpuoqvx f6950fc1 (empty) (no description set)
        Parent commit      : mwrttmos 8d698d4a my-bookmark | My commit message
        Added 1 files, modified 0 files, removed 0 files
        Initialized repo in "repo"
        "###);
    }

    let workspace_root = test_env.env_root().join("repo");
    let jj_path = workspace_root.join(".jj");
    let repo_path = jj_path.join("repo");
    let store_path = repo_path.join("store");
    assert!(workspace_root.is_dir());
    assert!(jj_path.is_dir());
    assert!(jj_path.join("working_copy").is_dir());
    assert!(repo_path.is_dir());
    assert!(store_path.is_dir());
    let unix_git_target_file_contents = read_git_target(&workspace_root).replace('\\', "/");
    if bare {
        assert!(unix_git_target_file_contents.ends_with("/git-repo"));
    } else {
        assert!(unix_git_target_file_contents.ends_with("/git-repo/.git"));
    }

    // Check that the Git repo's HEAD got checked out
    insta::allow_duplicates! {
        insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r#"
        @  f6950fc115ae
        ○  8d698d4a8ee1 my-bookmark git_head() My commit message
        ◆  000000000000
        "#);
    }
}

#[test_case(false; "full")]
#[test_case(true; "bare")]
fn test_git_init_external_import_trunk(bare: bool) {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_repo = init_git_repo(&git_repo_path, bare);

    // Add remote bookmark "trunk" for remote "origin", and set it as "origin/HEAD"
    let oid = git_repo
        .find_reference("refs/heads/my-bookmark")
        .unwrap()
        .target()
        .unwrap();
    git_repo
        .reference("refs/remotes/origin/trunk", oid, false, "")
        .unwrap();
    git_repo
        .reference_symbolic(
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/trunk",
            false,
            "",
        )
        .unwrap();

    let (stdout, stderr) = test_env.jj_cmd_ok(
        test_env.env_root(),
        &[
            "git",
            "init",
            "repo",
            "--git-repo",
            git_repo_path.to_str().unwrap(),
        ],
    );
    insta::allow_duplicates! {
        insta::assert_snapshot!(stdout, @"");
    }
    insta::allow_duplicates! {
        insta::assert_snapshot!(stderr, @r#"
        Done importing changes from the underlying Git repo.
        Setting the revset alias `trunk()` to `trunk@origin`
        Working copy now at: sqpuoqvx f6950fc1 (empty) (no description set)
        Parent commit      : mwrttmos 8d698d4a my-bookmark trunk@origin | My commit message
        Added 1 files, modified 0 files, removed 0 files
        Initialized repo in "repo"
        "#);
    }

    // "trunk()" alias should be set to remote "origin"'s default bookmark "trunk"
    let stdout = test_env.jj_cmd_success(
        &test_env.env_root().join("repo"),
        &["config", "list", "--repo", "revset-aliases.\"trunk()\""],
    );
    insta::allow_duplicates! {
        insta::assert_snapshot!(stdout, @r###"
        revset-aliases."trunk()" = "trunk@origin"
        "###);
    }
}

#[test]
fn test_git_init_external_ignore_working_copy() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    init_git_repo(&git_repo_path, false);
    let workspace_root = test_env.env_root().join("repo");
    std::fs::create_dir(&workspace_root).unwrap();
    std::fs::write(workspace_root.join("file1"), "").unwrap();

    // No snapshot should be taken
    let stderr = test_env.jj_cmd_cli_error(
        &workspace_root,
        &[
            "git",
            "init",
            "--ignore-working-copy",
            "--git-repo",
            git_repo_path.to_str().unwrap(),
        ],
    );
    insta::assert_snapshot!(stderr, @r###"
    Error: --ignore-working-copy is not respected
    "###);
}

#[test]
fn test_git_init_external_at_operation() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    init_git_repo(&git_repo_path, false);
    let workspace_root = test_env.env_root().join("repo");
    std::fs::create_dir(&workspace_root).unwrap();

    let stderr = test_env.jj_cmd_cli_error(
        &workspace_root,
        &[
            "git",
            "init",
            "--at-op=@-",
            "--git-repo",
            git_repo_path.to_str().unwrap(),
        ],
    );
    insta::assert_snapshot!(stderr, @r###"
    Error: --at-op is not respected
    "###);
}

#[test]
fn test_git_init_external_non_existent_directory() {
    let test_env = TestEnvironment::default();
    let stderr = test_env.jj_cmd_failure(
        test_env.env_root(),
        &["git", "init", "repo", "--git-repo", "non-existent"],
    );
    insta::assert_snapshot!(strip_last_line(&stderr), @r###"
    Error: Failed to access the repository
    Caused by:
    1: Cannot access $TEST_ENV/non-existent
    "###);
}

#[test]
fn test_git_init_external_non_existent_git_directory() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    let stderr = test_env.jj_cmd_failure(
        test_env.env_root(),
        &["git", "init", "repo", "--git-repo", "repo"],
    );

    insta::assert_snapshot!(&stderr, @r###"
    Error: Failed to access the repository
    Caused by:
    1: Failed to open git repository
    2: "$TEST_ENV/repo" does not appear to be a git repository
    3: Missing HEAD at '.git/HEAD'
    "###);
    let jj_path = workspace_root.join(".jj");
    assert!(!jj_path.exists());
}

#[test]
fn test_git_init_colocated_via_git_repo_path() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    init_git_repo(&workspace_root, false);
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    "###);

    let jj_path = workspace_root.join(".jj");
    let repo_path = jj_path.join("repo");
    let store_path = repo_path.join("store");
    assert!(workspace_root.is_dir());
    assert!(jj_path.is_dir());
    assert!(jj_path.join("working_copy").is_dir());
    assert!(repo_path.is_dir());
    assert!(store_path.is_dir());
    assert!(read_git_target(&workspace_root)
        .replace('\\', "/")
        .ends_with("../../../.git"));

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r#"
    @  f61b77cd4bb5
    ○  8d698d4a8ee1 my-bookmark git_head() My commit message
    ◆  000000000000
    "#);

    // Check that the Git repo's HEAD moves
    test_env.jj_cmd_ok(&workspace_root, &["new"]);
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r#"
    @  f1c7aa7c62d8
    ○  f61b77cd4bb5 git_head()
    ○  8d698d4a8ee1 my-bookmark My commit message
    ◆  000000000000
    "#);
}

#[test]
fn test_git_init_colocated_via_git_repo_path_gitlink() {
    let test_env = TestEnvironment::default();
    // <workspace_root>/.git -> <git_repo_path>
    let git_repo_path = test_env.env_root().join("git-repo");
    let workspace_root = test_env.env_root().join("repo");
    init_git_repo_with_opts(
        &git_repo_path,
        git2::RepositoryInitOptions::new().workdir_path(&workspace_root),
    );
    assert!(workspace_root.join(".git").is_file());
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    "###);
    insta::assert_snapshot!(read_git_target(&workspace_root), @"../../../.git");

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r#"
    @  f61b77cd4bb5
    ○  8d698d4a8ee1 my-bookmark git_head() My commit message
    ◆  000000000000
    "#);

    // Check that the Git repo's HEAD moves
    test_env.jj_cmd_ok(&workspace_root, &["new"]);
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r#"
    @  f1c7aa7c62d8
    ○  f61b77cd4bb5 git_head()
    ○  8d698d4a8ee1 my-bookmark My commit message
    ◆  000000000000
    "#);
}

#[cfg(unix)]
#[test]
fn test_git_init_colocated_via_git_repo_path_symlink_directory() {
    let test_env = TestEnvironment::default();
    // <workspace_root>/.git -> <git_repo_path>
    let git_repo_path = test_env.env_root().join("git-repo");
    let workspace_root = test_env.env_root().join("repo");
    init_git_repo(&git_repo_path, false);
    std::fs::create_dir(&workspace_root).unwrap();
    std::os::unix::fs::symlink(git_repo_path.join(".git"), workspace_root.join(".git")).unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    "###);
    insta::assert_snapshot!(read_git_target(&workspace_root), @"../../../.git");

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r#"
    @  f61b77cd4bb5
    ○  8d698d4a8ee1 my-bookmark git_head() My commit message
    ◆  000000000000
    "#);

    // Check that the Git repo's HEAD moves
    test_env.jj_cmd_ok(&workspace_root, &["new"]);
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r#"
    @  f1c7aa7c62d8
    ○  f61b77cd4bb5 git_head()
    ○  8d698d4a8ee1 my-bookmark My commit message
    ◆  000000000000
    "#);
}

#[cfg(unix)]
#[test]
fn test_git_init_colocated_via_git_repo_path_symlink_directory_without_bare_config() {
    let test_env = TestEnvironment::default();
    // <workspace_root>/.git -> <git_repo_path>
    let git_repo_path = test_env.env_root().join("git-repo.git");
    let workspace_root = test_env.env_root().join("repo");
    // Set up git repo without core.bare set (as the "repo" tool would do.)
    // The core.bare config is deduced from the directory name.
    let git_repo = init_git_repo(&workspace_root, false);
    git_repo.config().unwrap().remove("core.bare").unwrap();
    std::fs::rename(workspace_root.join(".git"), &git_repo_path).unwrap();
    std::os::unix::fs::symlink(&git_repo_path, workspace_root.join(".git")).unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    "###);
    insta::assert_snapshot!(read_git_target(&workspace_root), @"../../../.git");

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r#"
    @  f61b77cd4bb5
    ○  8d698d4a8ee1 my-bookmark git_head() My commit message
    ◆  000000000000
    "#);

    // Check that the Git repo's HEAD moves
    test_env.jj_cmd_ok(&workspace_root, &["new"]);
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r#"
    @  f1c7aa7c62d8
    ○  f61b77cd4bb5 git_head()
    ○  8d698d4a8ee1 my-bookmark My commit message
    ◆  000000000000
    "#);
}

#[cfg(unix)]
#[test]
fn test_git_init_colocated_via_git_repo_path_symlink_gitlink() {
    let test_env = TestEnvironment::default();
    // <workspace_root>/.git -> <git_workdir_path>/.git -> <git_repo_path>
    let git_repo_path = test_env.env_root().join("git-repo");
    let git_workdir_path = test_env.env_root().join("git-workdir");
    let workspace_root = test_env.env_root().join("repo");
    init_git_repo_with_opts(
        &git_repo_path,
        git2::RepositoryInitOptions::new().workdir_path(&git_workdir_path),
    );
    assert!(git_workdir_path.join(".git").is_file());
    std::fs::create_dir(&workspace_root).unwrap();
    std::os::unix::fs::symlink(git_workdir_path.join(".git"), workspace_root.join(".git")).unwrap();
    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    "###);
    insta::assert_snapshot!(read_git_target(&workspace_root), @"../../../.git");

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r#"
    @  f61b77cd4bb5
    ○  8d698d4a8ee1 my-bookmark git_head() My commit message
    ◆  000000000000
    "#);

    // Check that the Git repo's HEAD moves
    test_env.jj_cmd_ok(&workspace_root, &["new"]);
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r#"
    @  f1c7aa7c62d8
    ○  f61b77cd4bb5 git_head()
    ○  8d698d4a8ee1 my-bookmark My commit message
    ◆  000000000000
    "#);
}

#[test]
fn test_git_init_colocated_via_git_repo_path_imported_refs() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");

    // Set up remote refs
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "remote"]);
    let remote_path = test_env.env_root().join("remote");
    test_env.jj_cmd_ok(
        &remote_path,
        &["bookmark", "create", "local-remote", "remote-only"],
    );
    test_env.jj_cmd_ok(&remote_path, &["new"]);
    test_env.jj_cmd_ok(&remote_path, &["git", "export"]);

    let remote_git_path = remote_path.join(PathBuf::from_iter([".jj", "repo", "store", "git"]));
    let set_up_local_repo = |local_path: &Path| {
        let git_repo =
            git2::Repository::clone(remote_git_path.to_str().unwrap(), local_path).unwrap();
        let git_ref = git_repo
            .find_reference("refs/remotes/origin/local-remote")
            .unwrap();
        git_repo
            .reference(
                "refs/heads/local-remote",
                git_ref.target().unwrap(),
                false,
                "",
            )
            .unwrap();
    };

    // With git.auto-local-bookmark = true
    let local_path = test_env.env_root().join("local1");
    set_up_local_repo(&local_path);
    let (_stdout, stderr) = test_env.jj_cmd_ok(&local_path, &["git", "init", "--git-repo=."]);
    insta::assert_snapshot!(stderr, @r###"
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    "###);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &local_path), @r###"
    local-remote: vvkvtnvv 230dd059 (empty) (no description set)
      @git: vvkvtnvv 230dd059 (empty) (no description set)
      @origin: vvkvtnvv 230dd059 (empty) (no description set)
    remote-only: vvkvtnvv 230dd059 (empty) (no description set)
      @git: vvkvtnvv 230dd059 (empty) (no description set)
      @origin: vvkvtnvv 230dd059 (empty) (no description set)
    "###);

    // With git.auto-local-bookmark = false
    test_env.add_config("git.auto-local-bookmark = false");
    let local_path = test_env.env_root().join("local2");
    set_up_local_repo(&local_path);
    let (_stdout, stderr) = test_env.jj_cmd_ok(&local_path, &["git", "init", "--git-repo=."]);
    insta::assert_snapshot!(stderr, @r###"
    Done importing changes from the underlying Git repo.
    Hint: The following remote bookmarks aren't associated with the existing local bookmarks:
      local-remote@origin
    Hint: Run `jj bookmark track local-remote@origin` to keep local bookmarks updated on future pulls.
    Initialized repo in "."
    "###);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &local_path), @r###"
    local-remote: vvkvtnvv 230dd059 (empty) (no description set)
      @git: vvkvtnvv 230dd059 (empty) (no description set)
    local-remote@origin: vvkvtnvv 230dd059 (empty) (no description set)
    remote-only@origin: vvkvtnvv 230dd059 (empty) (no description set)
    "###);
}

#[test]
fn test_git_init_colocated_dirty_working_copy() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    let git_repo = init_git_repo(&workspace_root, false);

    let add_file_to_index = |name: &str, data: &str| {
        std::fs::write(workspace_root.join(name), data).unwrap();
        let mut index = git_repo.index().unwrap();
        index.add_path(Path::new(name)).unwrap();
        index.write().unwrap();
    };
    let get_git_statuses = || {
        let mut buf = String::new();
        for entry in git_repo.statuses(None).unwrap().iter() {
            writeln!(buf, "{:?} {}", entry.status(), entry.path().unwrap()).unwrap();
        }
        buf
    };

    add_file_to_index("some-file", "new content");
    add_file_to_index("new-staged-file", "new content");
    std::fs::write(workspace_root.join("unstaged-file"), "new content").unwrap();
    insta::assert_snapshot!(get_git_statuses(), @r###"
    Status(INDEX_NEW) new-staged-file
    Status(INDEX_MODIFIED) some-file
    Status(WT_NEW) unstaged-file
    "###);

    let (stdout, stderr) = test_env.jj_cmd_ok(&workspace_root, &["git", "init", "--git-repo", "."]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Done importing changes from the underlying Git repo.
    Initialized repo in "."
    "###);

    // Working-copy changes should have been snapshotted.
    let stdout = test_env.jj_cmd_success(&workspace_root, &["log", "-s", "--ignore-working-copy"]);
    insta::assert_snapshot!(stdout, @r#"
    @  sqpuoqvx test.user@example.com 2001-02-03 08:05:07 cd1e144d
    │  (no description set)
    │  C {some-file => new-staged-file}
    │  M some-file
    │  C {some-file => unstaged-file}
    ○  mwrttmos git.user@example.com 1970-01-01 11:02:03 my-bookmark git_head() 8d698d4a
    │  My commit message
    │  A some-file
    ◆  zzzzzzzz root() 00000000
    "#);

    // Git index should be consistent with the working copy parent. With the
    // current implementation, the index is unchanged. Since jj created new
    // working copy commit, it's also okay to update the index reflecting the
    // working copy commit or the working copy parent.
    insta::assert_snapshot!(get_git_statuses(), @r###"
    Status(IGNORED) .jj/.gitignore
    Status(IGNORED) .jj/repo/
    Status(IGNORED) .jj/working_copy/
    Status(INDEX_NEW) new-staged-file
    Status(INDEX_MODIFIED) some-file
    Status(WT_NEW) unstaged-file
    "###);
}

#[test]
fn test_git_init_colocated_ignore_working_copy() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    init_git_repo(&workspace_root, false);
    std::fs::write(workspace_root.join("file1"), "").unwrap();

    let stderr = test_env.jj_cmd_cli_error(
        &workspace_root,
        &["git", "init", "--ignore-working-copy", "--colocate"],
    );
    insta::assert_snapshot!(stderr, @r###"
    Error: --ignore-working-copy is not respected
    "###);
}

#[test]
fn test_git_init_colocated_at_operation() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    init_git_repo(&workspace_root, false);

    let stderr = test_env.jj_cmd_cli_error(
        &workspace_root,
        &["git", "init", "--at-op=@-", "--colocate"],
    );
    insta::assert_snapshot!(stderr, @r###"
    Error: --at-op is not respected
    "###);
}

#[test]
fn test_git_init_external_but_git_dir_exists() {
    let test_env = TestEnvironment::default();
    let git_repo_path = test_env.env_root().join("git-repo");
    let workspace_root = test_env.env_root().join("repo");
    git2::Repository::init(&git_repo_path).unwrap();
    init_git_repo(&workspace_root, false);
    let (stdout, stderr) = test_env.jj_cmd_ok(
        &workspace_root,
        &["git", "init", "--git-repo", git_repo_path.to_str().unwrap()],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Initialized repo in "."
    "###);

    // The local ".git" repository is unrelated, so no commits should be imported
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r###"
    @  230dd059e1b0
    ◆  000000000000
    "###);

    // Check that Git HEAD is not set because this isn't a colocated repo
    test_env.jj_cmd_ok(&workspace_root, &["new"]);
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r###"
    @  4db490c88528
    ○  230dd059e1b0
    ◆  000000000000
    "###);
}

#[test]
fn test_git_init_colocated_via_flag_git_dir_exists() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    init_git_repo(&workspace_root, false);

    let (stdout, stderr) =
        test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "--colocate", "repo"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Done importing changes from the underlying Git repo.
    Initialized repo in "repo"
    "###);

    // Check that the Git repo's HEAD got checked out
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r#"
    @  f61b77cd4bb5
    ○  8d698d4a8ee1 my-bookmark git_head() My commit message
    ◆  000000000000
    "#);

    // Check that the Git repo's HEAD moves
    test_env.jj_cmd_ok(&workspace_root, &["new"]);
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r#"
    @  f1c7aa7c62d8
    ○  f61b77cd4bb5 git_head()
    ○  8d698d4a8ee1 my-bookmark My commit message
    ◆  000000000000
    "#);
}

#[test]
fn test_git_init_colocated_via_flag_git_dir_not_exists() {
    let test_env = TestEnvironment::default();
    let workspace_root = test_env.env_root().join("repo");
    let (stdout, stderr) =
        test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "--colocate", "repo"]);
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r###"
    Initialized repo in "repo"
    "###);
    // No HEAD ref is available yet
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r###"
    @  230dd059e1b0
    ◆  000000000000
    "###);

    // Create the default bookmark (create both in case we change the default)
    test_env.jj_cmd_ok(&workspace_root, &["bookmark", "create", "main", "master"]);

    // If .git/HEAD pointed to the default bookmark, new working-copy commit would
    // be created on top.
    insta::assert_snapshot!(get_log_output(&test_env, &workspace_root), @r###"
    @  230dd059e1b0 main master
    ◆  000000000000
    "###);
}

#[test]
fn test_git_init_conditional_config() {
    let test_env = TestEnvironment::default();
    let old_workspace_root = test_env.env_root().join("old");
    let new_workspace_root = test_env.env_root().join("new");

    let jj_cmd_ok = |current_dir: &Path, args: &[&str]| {
        let mut cmd = test_env.jj_cmd(current_dir, args);
        cmd.env_remove("JJ_EMAIL");
        cmd.env_remove("JJ_OP_HOSTNAME");
        cmd.env_remove("JJ_OP_USERNAME");
        let assert = cmd.assert().success();
        let stdout = test_env.normalize_output(&get_stdout_string(&assert));
        let stderr = test_env.normalize_output(&get_stderr_string(&assert));
        (stdout, stderr)
    };
    let log_template = r#"separate(' ', author.email(), description.first_line()) ++ "\n""#;
    let op_log_template = r#"separate(' ', user, description.first_line()) ++ "\n""#;

    // Override user.email and operation.username conditionally
    test_env.add_config(formatdoc! {"
        user.email = 'base@example.org'
        operation.hostname = 'base'
        operation.username = 'base'
        [[--scope]]
        --when.repositories = [{new_workspace_root}]
        user.email = 'new-repo@example.org'
        operation.username = 'new-repo'
        ",
        new_workspace_root = to_toml_value(new_workspace_root.to_str().unwrap()),
    });

    // Override operation.hostname by repo config, which should be loaded into
    // the command settings, but shouldn't be copied to the new repo.
    jj_cmd_ok(test_env.env_root(), &["git", "init", "old"]);
    jj_cmd_ok(
        &old_workspace_root,
        &["config", "set", "--repo", "operation.hostname", "old-repo"],
    );
    jj_cmd_ok(&old_workspace_root, &["new"]);
    let (stdout, _stderr) = jj_cmd_ok(&old_workspace_root, &["op", "log", "-T", op_log_template]);
    insta::assert_snapshot!(stdout, @r"
    @  base@old-repo new empty commit
    ○  base@base add workspace 'default'
    ○  @
    ");

    // Create new repo at the old workspace directory.
    let (_stdout, stderr) = jj_cmd_ok(&old_workspace_root, &["git", "init", "../new"]);
    insta::assert_snapshot!(stderr.replace('\\', "/"), @r#"Initialized repo in "../new""#);
    jj_cmd_ok(&new_workspace_root, &["new"]);
    let (stdout, _stderr) = jj_cmd_ok(&new_workspace_root, &["log", "-T", log_template]);
    insta::assert_snapshot!(stdout, @r"
    @  new-repo@example.org
    ○  new-repo@example.org
    ◆
    ");
    let (stdout, _stderr) = jj_cmd_ok(&new_workspace_root, &["op", "log", "-T", op_log_template]);
    insta::assert_snapshot!(stdout, @r"
    @  new-repo@base new empty commit
    ○  new-repo@base add workspace 'default'
    ○  @
    ");
}

#[test]
fn test_git_init_bad_wc_path() {
    let test_env = TestEnvironment::default();
    std::fs::write(test_env.env_root().join("existing-file"), b"").unwrap();
    let stderr = test_env.jj_cmd_failure(test_env.env_root(), &["git", "init", "existing-file"]);
    assert!(stderr.contains("Failed to create workspace"));
}
