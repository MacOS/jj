// Copyright 2022 The Jujutsu Authors
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

use std::path;
use std::path::Path;
use std::path::PathBuf;

use indoc::formatdoc;
use test_case::test_case;

use crate::common::get_stderr_string;
use crate::common::get_stdout_string;
use crate::common::strip_last_line;
use crate::common::to_toml_value;
use crate::common::TestEnvironment;

fn set_up_non_empty_git_repo(git_repo: &git2::Repository) {
    set_up_git_repo_with_file(git_repo, "file");
}

fn set_up_git_repo_with_file(git_repo: &git2::Repository, filename: &str) {
    let signature =
        git2::Signature::new("Some One", "some.one@example.com", &git2::Time::new(0, 0)).unwrap();
    let mut tree_builder = git_repo.treebuilder(None).unwrap();
    let file_oid = git_repo.blob(b"content").unwrap();
    tree_builder
        .insert(filename, file_oid, git2::FileMode::Blob.into())
        .unwrap();
    let tree_oid = tree_builder.write().unwrap();
    let tree = git_repo.find_tree(tree_oid).unwrap();
    git_repo
        .commit(
            Some("refs/heads/main"),
            &signature,
            &signature,
            "message",
            &tree,
            &[],
        )
        .unwrap();
    git_repo.set_head("refs/heads/main").unwrap();
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone(subprocess: bool) {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    if subprocess {
        test_env.add_config("git.subprocess = true");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git2::Repository::init(git_repo_path).unwrap();

    // Clone an empty repo
    let (stdout, stderr) =
        test_env.jj_cmd_ok(test_env.env_root(), &["git", "clone", "source", "empty"]);
    insta::allow_duplicates! { insta::assert_snapshot!(stdout, @""); }
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r###"
    Fetching into new repo in "$TEST_ENV/empty"
    Nothing changed.
    "###);
    }

    set_up_non_empty_git_repo(&git_repo);

    // Clone with relative source path
    let (stdout, stderr) =
        test_env.jj_cmd_ok(test_env.env_root(), &["git", "clone", "source", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @"");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r#"
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: uuqppmxq 1f0b881a (empty) (no description set)
    Parent commit      : mzyxwzks 9f01a0e0 main | message
    Added 1 files, modified 0 files, removed 0 files
    "#);
    }
    assert!(test_env.env_root().join("clone").join("file").exists());

    // Subsequent fetch should just work even if the source path was relative
    let (stdout, stderr) =
        test_env.jj_cmd_ok(&test_env.env_root().join("clone"), &["git", "fetch"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @"");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r###"
    Nothing changed.
    "###);
    }

    // Failed clone should clean up the destination directory
    std::fs::create_dir(test_env.env_root().join("bad")).unwrap();
    let assert = test_env
        .jj_cmd(test_env.env_root(), &["git", "clone", "bad", "failed"])
        .assert()
        .code(1);
    let stdout = test_env.normalize_output(&get_stdout_string(&assert));
    let stderr = test_env.normalize_output(&get_stderr_string(&assert));
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @"");
    }
    // git2's internal error is slightly different
    if subprocess {
        insta::assert_snapshot!(stderr, @r#"
        Fetching into new repo in "$TEST_ENV/failed"
        Error: Could not find repository at '$TEST_ENV/bad'
        "#);
    } else {
        insta::assert_snapshot!(stderr, @r#"
        Fetching into new repo in "$TEST_ENV/failed"
        Error: could not find repository at '$TEST_ENV/bad'; class=Repository (6)
        "#);
    }
    assert!(!test_env.env_root().join("failed").exists());

    // Failed clone shouldn't remove the existing destination directory
    std::fs::create_dir(test_env.env_root().join("failed")).unwrap();
    let assert = test_env
        .jj_cmd(test_env.env_root(), &["git", "clone", "bad", "failed"])
        .assert()
        .code(1);
    let stdout = test_env.normalize_output(&get_stdout_string(&assert));
    let stderr = test_env.normalize_output(&get_stderr_string(&assert));
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @"");
    }
    // git2's internal error is slightly different
    if subprocess {
        insta::assert_snapshot!(stderr, @r#"
        Fetching into new repo in "$TEST_ENV/failed"
        Error: Could not find repository at '$TEST_ENV/bad'
        "#);
    } else {
        insta::assert_snapshot!(stderr, @r#"
        Fetching into new repo in "$TEST_ENV/failed"
        Error: could not find repository at '$TEST_ENV/bad'; class=Repository (6)
        "#);
    }
    assert!(test_env.env_root().join("failed").exists());
    assert!(!test_env.env_root().join("failed").join(".jj").exists());

    // Failed clone (if attempted) shouldn't remove the existing workspace
    let stderr = test_env.jj_cmd_failure(test_env.env_root(), &["git", "clone", "bad", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r###"
    Error: Destination path exists and is not an empty directory
    "###);
    }
    assert!(test_env.env_root().join("clone").join(".jj").exists());

    // Try cloning into an existing workspace
    let stderr = test_env.jj_cmd_failure(test_env.env_root(), &["git", "clone", "source", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r###"
    Error: Destination path exists and is not an empty directory
    "###);
    }

    // Try cloning into an existing file
    std::fs::write(test_env.env_root().join("file"), "contents").unwrap();
    let stderr = test_env.jj_cmd_failure(test_env.env_root(), &["git", "clone", "source", "file"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r###"
    Error: Destination path exists and is not an empty directory
    "###);
    }

    // Try cloning into non-empty, non-workspace directory
    std::fs::remove_dir_all(test_env.env_root().join("clone").join(".jj")).unwrap();
    let stderr = test_env.jj_cmd_failure(test_env.env_root(), &["git", "clone", "source", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r###"
    Error: Destination path exists and is not an empty directory
    "###);
    }

    // Clone into a nested path
    let (stdout, stderr) = test_env.jj_cmd_ok(
        test_env.env_root(),
        &["git", "clone", "source", "nested/path/to/repo"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @"");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r#"
    Fetching into new repo in "$TEST_ENV/nested/path/to/repo"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: uuzqqzqu df8acbac (empty) (no description set)
    Parent commit      : mzyxwzks 9f01a0e0 main | message
    Added 1 files, modified 0 files, removed 0 files
    "#);
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_bad_source(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if subprocess {
        test_env.add_config("git.subprocess = true");
    }

    let stderr = test_env.jj_cmd_cli_error(test_env.env_root(), &["git", "clone", "", "dest"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r#"Error: local path "" does not specify a path to a repository"#);
    }

    // Invalid port number
    let stderr = test_env.jj_cmd_cli_error(
        test_env.env_root(),
        &["git", "clone", "https://example.net:bad-port/bar", "dest"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r#"
    Error: URL "https://example.net:bad-port/bar" can not be parsed as valid URL
    Caused by: invalid port number
    "#);
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_colocate(subprocess: bool) {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    if subprocess {
        test_env.add_config("git.subprocess = true");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git2::Repository::init(git_repo_path).unwrap();

    // Clone an empty repo
    let (stdout, stderr) = test_env.jj_cmd_ok(
        test_env.env_root(),
        &["git", "clone", "source", "empty", "--colocate"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @"");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r###"
    Fetching into new repo in "$TEST_ENV/empty"
    Nothing changed.
    "###);
    }

    // git_target path should be relative to the store
    let store_path = test_env
        .env_root()
        .join(PathBuf::from_iter(["empty", ".jj", "repo", "store"]));
    let git_target_file_contents = std::fs::read_to_string(store_path.join("git_target")).unwrap();
    insta::allow_duplicates! {
    insta::assert_snapshot!(
        git_target_file_contents.replace(path::MAIN_SEPARATOR, "/"),
        @"../../../.git");
    }

    set_up_non_empty_git_repo(&git_repo);

    // Clone with relative source path
    let (stdout, stderr) = test_env.jj_cmd_ok(
        test_env.env_root(),
        &["git", "clone", "source", "clone", "--colocate"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @"");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r#"
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: uuqppmxq 1f0b881a (empty) (no description set)
    Parent commit      : mzyxwzks 9f01a0e0 main | message
    Added 1 files, modified 0 files, removed 0 files
    "#);
    }
    assert!(test_env.env_root().join("clone").join("file").exists());
    assert!(test_env.env_root().join("clone").join(".git").exists());

    eprintln!(
        "{:?}",
        git_repo.head().expect("Repo head should be set").name()
    );

    let jj_git_repo = git2::Repository::open(test_env.env_root().join("clone"))
        .expect("Could not open clone repo");
    assert_eq!(
        jj_git_repo
            .head()
            .expect("Clone Repo HEAD should be set.")
            .symbolic_target(),
        git_repo
            .head()
            .expect("Repo HEAD should be set.")
            .symbolic_target()
    );
    // ".jj" directory should be ignored at Git side.
    #[allow(clippy::format_collect)]
    let git_statuses: String = jj_git_repo
        .statuses(None)
        .unwrap()
        .iter()
        .map(|entry| format!("{:?} {}\n", entry.status(), entry.path().unwrap()))
        .collect();
    insta::allow_duplicates! {
    insta::assert_snapshot!(git_statuses, @r###"
    Status(IGNORED) .jj/.gitignore
    Status(IGNORED) .jj/repo/
    Status(IGNORED) .jj/working_copy/
    "###);
    }

    // The old default bookmark "master" shouldn't exist.
    insta::allow_duplicates! {
    insta::assert_snapshot!(
        get_bookmark_output(&test_env, &test_env.env_root().join("clone")), @r###"
    main: mzyxwzks 9f01a0e0 message
      @git: mzyxwzks 9f01a0e0 message
      @origin: mzyxwzks 9f01a0e0 message
    "###);
    }

    // Subsequent fetch should just work even if the source path was relative
    let (stdout, stderr) =
        test_env.jj_cmd_ok(&test_env.env_root().join("clone"), &["git", "fetch"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @"");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r###"
    Nothing changed.
    "###);
    }

    // Failed clone should clean up the destination directory
    std::fs::create_dir(test_env.env_root().join("bad")).unwrap();
    let assert = test_env
        .jj_cmd(
            test_env.env_root(),
            &["git", "clone", "--colocate", "bad", "failed"],
        )
        .assert()
        .code(1);
    let stdout = test_env.normalize_output(&get_stdout_string(&assert));
    let stderr = test_env.normalize_output(&get_stderr_string(&assert));
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @"");
    }
    // git2's internal error is slightly different
    if subprocess {
        insta::assert_snapshot!(stderr, @r#"
        Fetching into new repo in "$TEST_ENV/failed"
        Error: Could not find repository at '$TEST_ENV/bad'
        "#);
    } else {
        insta::assert_snapshot!(stderr, @r#"
        Fetching into new repo in "$TEST_ENV/failed"
        Error: could not find repository at '$TEST_ENV/bad'; class=Repository (6)
        "#);
    }
    assert!(!test_env.env_root().join("failed").exists());

    // Failed clone shouldn't remove the existing destination directory
    std::fs::create_dir(test_env.env_root().join("failed")).unwrap();
    let assert = test_env
        .jj_cmd(
            test_env.env_root(),
            &["git", "clone", "--colocate", "bad", "failed"],
        )
        .assert()
        .code(1);
    let stdout = test_env.normalize_output(&get_stdout_string(&assert));
    let stderr = test_env.normalize_output(&get_stderr_string(&assert));
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @"");
    }
    // git2's internal error is slightly different
    if subprocess {
        insta::assert_snapshot!(stderr, @r#"
        Fetching into new repo in "$TEST_ENV/failed"
        Error: Could not find repository at '$TEST_ENV/bad'
        "#);
    } else {
        insta::assert_snapshot!(stderr, @r#"
        Fetching into new repo in "$TEST_ENV/failed"
        Error: could not find repository at '$TEST_ENV/bad'; class=Repository (6)
        "#);
    }
    assert!(test_env.env_root().join("failed").exists());
    assert!(!test_env.env_root().join("failed").join(".git").exists());
    assert!(!test_env.env_root().join("failed").join(".jj").exists());

    // Failed clone (if attempted) shouldn't remove the existing workspace
    let stderr = test_env.jj_cmd_failure(
        test_env.env_root(),
        &["git", "clone", "--colocate", "bad", "clone"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r###"
    Error: Destination path exists and is not an empty directory
    "###);
    }
    assert!(test_env.env_root().join("clone").join(".git").exists());
    assert!(test_env.env_root().join("clone").join(".jj").exists());

    // Try cloning into an existing workspace
    let stderr = test_env.jj_cmd_failure(
        test_env.env_root(),
        &["git", "clone", "source", "clone", "--colocate"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r###"
    Error: Destination path exists and is not an empty directory
    "###);
    }

    // Try cloning into an existing file
    std::fs::write(test_env.env_root().join("file"), "contents").unwrap();
    let stderr = test_env.jj_cmd_failure(
        test_env.env_root(),
        &["git", "clone", "source", "file", "--colocate"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r###"
    Error: Destination path exists and is not an empty directory
    "###);
    }

    // Try cloning into non-empty, non-workspace directory
    std::fs::remove_dir_all(test_env.env_root().join("clone").join(".jj")).unwrap();
    let stderr = test_env.jj_cmd_failure(
        test_env.env_root(),
        &["git", "clone", "source", "clone", "--colocate"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r###"
    Error: Destination path exists and is not an empty directory
    "###);
    }

    // Clone into a nested path
    let (stdout, stderr) = test_env.jj_cmd_ok(
        test_env.env_root(),
        &[
            "git",
            "clone",
            "source",
            "nested/path/to/repo",
            "--colocate",
        ],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @"");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r#"
    Fetching into new repo in "$TEST_ENV/nested/path/to/repo"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: vzqnnsmr 9407107f (empty) (no description set)
    Parent commit      : mzyxwzks 9f01a0e0 main | message
    Added 1 files, modified 0 files, removed 0 files
    "#);
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_remote_default_bookmark(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if subprocess {
        test_env.add_config("git.subprocess = true");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git2::Repository::init(git_repo_path).unwrap();
    set_up_non_empty_git_repo(&git_repo);
    // Create non-default bookmark in remote
    let oid = git_repo
        .find_reference("refs/heads/main")
        .unwrap()
        .target()
        .unwrap();
    git_repo
        .reference("refs/heads/feature1", oid, false, "")
        .unwrap();

    // All fetched bookmarks will be imported if auto-local-bookmark is on
    test_env.add_config("git.auto-local-bookmark = true");
    let (_stdout, stderr) =
        test_env.jj_cmd_ok(test_env.env_root(), &["git", "clone", "source", "clone1"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r#"
    Fetching into new repo in "$TEST_ENV/clone1"
    bookmark: feature1@origin [new] tracked
    bookmark: main@origin     [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: sqpuoqvx cad212e1 (empty) (no description set)
    Parent commit      : mzyxwzks 9f01a0e0 feature1 main | message
    Added 1 files, modified 0 files, removed 0 files
    "#);
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(
        get_bookmark_output(&test_env, &test_env.env_root().join("clone1")), @r###"
    feature1: mzyxwzks 9f01a0e0 message
      @origin: mzyxwzks 9f01a0e0 message
    main: mzyxwzks 9f01a0e0 message
      @origin: mzyxwzks 9f01a0e0 message
    "###);
    }

    // "trunk()" alias should be set to default bookmark "main"
    let stdout = test_env.jj_cmd_success(
        &test_env.env_root().join("clone1"),
        &["config", "list", "--repo", "revset-aliases.'trunk()'"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @r###"
    revset-aliases.'trunk()' = "main@origin"
    "###);
    }

    // Only the default bookmark will be imported if auto-local-bookmark is off
    test_env.add_config("git.auto-local-bookmark = false");
    let (_stdout, stderr) =
        test_env.jj_cmd_ok(test_env.env_root(), &["git", "clone", "source", "clone2"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r#"
    Fetching into new repo in "$TEST_ENV/clone2"
    bookmark: feature1@origin [new] untracked
    bookmark: main@origin     [new] untracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: rzvqmyuk cc8a5041 (empty) (no description set)
    Parent commit      : mzyxwzks 9f01a0e0 feature1@origin main | message
    Added 1 files, modified 0 files, removed 0 files
    "#);
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(
        get_bookmark_output(&test_env, &test_env.env_root().join("clone2")), @r###"
    feature1@origin: mzyxwzks 9f01a0e0 message
    main: mzyxwzks 9f01a0e0 message
      @origin: mzyxwzks 9f01a0e0 message
    "###);
    }

    // Change the default bookmark in remote
    git_repo.set_head("refs/heads/feature1").unwrap();
    let (_stdout, stderr) =
        test_env.jj_cmd_ok(test_env.env_root(), &["git", "clone", "source", "clone3"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r#"
    Fetching into new repo in "$TEST_ENV/clone3"
    bookmark: feature1@origin [new] untracked
    bookmark: main@origin     [new] untracked
    Setting the revset alias `trunk()` to `feature1@origin`
    Working copy now at: nppvrztz b8a8a17b (empty) (no description set)
    Parent commit      : mzyxwzks 9f01a0e0 feature1 main@origin | message
    Added 1 files, modified 0 files, removed 0 files
    "#);
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(
        get_bookmark_output(&test_env, &test_env.env_root().join("clone3")), @r"
    feature1: mzyxwzks 9f01a0e0 message
      @origin: mzyxwzks 9f01a0e0 message
    main@origin: mzyxwzks 9f01a0e0 message
    ");
    }

    // "trunk()" alias should be set to new default bookmark "feature1"
    let stdout = test_env.jj_cmd_success(
        &test_env.env_root().join("clone3"),
        &["config", "list", "--repo", "revset-aliases.'trunk()'"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @r###"
    revset-aliases.'trunk()' = "feature1@origin"
    "###);
    }
}

// A branch with a strange name should get quoted in the config. Windows doesn't
// like the strange name, so we don't run the test there.
#[cfg(unix)]
#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_remote_default_bookmark_with_escape(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if subprocess {
        test_env.add_config("git.subprocess = true");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git2::Repository::init(git_repo_path).unwrap();
    set_up_non_empty_git_repo(&git_repo);
    // Rename the main branch to something that needs to be escaped
    git_repo
        .find_reference("refs/heads/main")
        .unwrap()
        .rename("refs/heads/\"", false, "")
        .unwrap();

    let (_stdout, stderr) =
        test_env.jj_cmd_ok(test_env.env_root(), &["git", "clone", "source", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r#"
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: "@origin [new] untracked
    Setting the revset alias `trunk()` to `"\""@origin`
    Working copy now at: sqpuoqvx cad212e1 (empty) (no description set)
    Parent commit      : mzyxwzks 9f01a0e0 " | message
    Added 1 files, modified 0 files, removed 0 files
    "#);
    }

    // "trunk()" alias should be escaped and quoted
    let stdout = test_env.jj_cmd_success(
        &test_env.env_root().join("clone"),
        &["config", "list", "--repo", "revset-aliases.'trunk()'"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @r#"revset-aliases.'trunk()' = '"\""@origin'"#);
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_ignore_working_copy(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if subprocess {
        test_env.add_config("git.subprocess = true");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git2::Repository::init(git_repo_path).unwrap();
    set_up_non_empty_git_repo(&git_repo);

    // Should not update working-copy files
    let (_stdout, stderr) = test_env.jj_cmd_ok(
        test_env.env_root(),
        &["git", "clone", "--ignore-working-copy", "source", "clone"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r#"
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] untracked
    Setting the revset alias `trunk()` to `main@origin`
    "#);
    }
    let clone_path = test_env.env_root().join("clone");

    let (stdout, stderr) = test_env.jj_cmd_ok(&clone_path, &["status", "--ignore-working-copy"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @r###"
    The working copy has no changes.
    Working copy : sqpuoqvx cad212e1 (empty) (no description set)
    Parent commit: mzyxwzks 9f01a0e0 main | message
    "###);
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @"");
    }

    // TODO: Correct, but might be better to check out the root commit?
    let stderr = test_env.jj_cmd_failure(&clone_path, &["status"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r##"
    Error: The working copy is stale (not updated since operation eac759b9ab75).
    Hint: Run `jj workspace update-stale` to update it.
    See https://jj-vcs.github.io/jj/latest/working-copy/#stale-working-copy for more information.
    "##);
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_at_operation(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if subprocess {
        test_env.add_config("git.subprocess = true");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git2::Repository::init(git_repo_path).unwrap();
    set_up_non_empty_git_repo(&git_repo);

    let stderr = test_env.jj_cmd_cli_error(
        test_env.env_root(),
        &["git", "clone", "--at-op=@-", "source", "clone"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r###"
    Error: --at-op is not respected
    "###);
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_with_remote_name(subprocess: bool) {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    if subprocess {
        test_env.add_config("git.subprocess = true");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git2::Repository::init(git_repo_path).unwrap();
    set_up_non_empty_git_repo(&git_repo);

    // Clone with relative source path and a non-default remote name
    let (stdout, stderr) = test_env.jj_cmd_ok(
        test_env.env_root(),
        &["git", "clone", "source", "clone", "--remote", "upstream"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @"");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r#"
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@upstream [new] tracked
    Setting the revset alias `trunk()` to `main@upstream`
    Working copy now at: sqpuoqvx cad212e1 (empty) (no description set)
    Parent commit      : mzyxwzks 9f01a0e0 main | message
    Added 1 files, modified 0 files, removed 0 files
    "#);
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_with_remote_named_git(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if subprocess {
        test_env.add_config("git.subprocess = true");
    }
    let git_repo_path = test_env.env_root().join("source");
    git2::Repository::init(git_repo_path).unwrap();

    let stderr = test_env.jj_cmd_failure(
        test_env.env_root(),
        &["git", "clone", "--remote=git", "source", "dest"],
    );
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @"Error: Git remote named 'git' is reserved for local Git repository");
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_trunk_deleted(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if subprocess {
        test_env.add_config("git.subprocess = true");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git2::Repository::init(git_repo_path).unwrap();
    set_up_non_empty_git_repo(&git_repo);
    let clone_path = test_env.env_root().join("clone");

    let (stdout, stderr) =
        test_env.jj_cmd_ok(test_env.env_root(), &["git", "clone", "source", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @"");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r#"
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] untracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: sqpuoqvx cad212e1 (empty) (no description set)
    Parent commit      : mzyxwzks 9f01a0e0 main | message
    Added 1 files, modified 0 files, removed 0 files
    "#);
    }

    let (stdout, stderr) = test_env.jj_cmd_ok(&clone_path, &["bookmark", "forget", "main"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @"");
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r"
    Forgot 1 bookmarks.
    Warning: Failed to resolve `revset-aliases.trunk()`: Revision `main@origin` doesn't exist
    Hint: Use `jj config edit --repo` to adjust the `trunk()` alias.
    ");
    }

    let (stdout, stderr) = test_env.jj_cmd_ok(&clone_path, &["log"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @r#"
    @  sqpuoqvx test.user@example.com 2001-02-03 08:05:07 cad212e1
    │  (empty) (no description set)
    ○  mzyxwzks some.one@example.com 1970-01-01 11:00:00 9f01a0e0
    │  message
    ◆  zzzzzzzz root() 00000000
    "#);
    }
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r"
    Warning: Failed to resolve `revset-aliases.trunk()`: Revision `main@origin` doesn't exist
    Hint: Use `jj config edit --repo` to adjust the `trunk()` alias.
    ");
    }
}

#[test]
fn test_git_clone_conditional_config() {
    let test_env = TestEnvironment::default();
    let source_repo_path = test_env.env_root().join("source");
    let old_workspace_root = test_env.env_root().join("old");
    let new_workspace_root = test_env.env_root().join("new");
    let source_git_repo = git2::Repository::init(source_repo_path).unwrap();
    set_up_non_empty_git_repo(&source_git_repo);

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

    // Clone repo at the old workspace directory.
    let (_stdout, stderr) = jj_cmd_ok(
        &old_workspace_root,
        &["git", "clone", "../source", "../new"],
    );
    insta::assert_snapshot!(stderr, @r#"
    Fetching into new repo in "$TEST_ENV/new"
    bookmark: main@origin [new] untracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: zxsnswpr 5695b5e5 (empty) (no description set)
    Parent commit      : mzyxwzks 9f01a0e0 main | message
    Added 1 files, modified 0 files, removed 0 files
    "#);
    jj_cmd_ok(&new_workspace_root, &["new"]);
    let (stdout, _stderr) = jj_cmd_ok(&new_workspace_root, &["log", "-T", log_template]);
    insta::assert_snapshot!(stdout, @r"
    @  new-repo@example.org
    ○  new-repo@example.org
    ◆  some.one@example.com message
    │
    ~
    ");
    let (stdout, _stderr) = jj_cmd_ok(&new_workspace_root, &["op", "log", "-T", op_log_template]);
    insta::assert_snapshot!(stdout, @r"
    @  new-repo@base new empty commit
    ○  new-repo@base check out git remote's default branch
    ○  new-repo@base fetch from git remote into empty repo
    ○  new-repo@base add workspace 'default'
    ○  @
    ");
}

#[test]
fn test_git_clone_with_depth_git2() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git2::Repository::init(git_repo_path).unwrap();
    set_up_non_empty_git_repo(&git_repo);

    // git does support shallow clones on the local transport, so it will work
    // (we cannot replicate git2's erroneous behaviour wrt git)
    // local transport does not support shallow clones so we just test that the
    // depth arg is passed on here
    let stderr = test_env.jj_cmd_failure(
        test_env.env_root(),
        &["git", "clone", "--depth", "1", "source", "clone"],
    );
    insta::assert_snapshot!(stderr, @r#"
    Fetching into new repo in "$TEST_ENV/clone"
    Error: shallow fetch is not supported by the local transport; class=Net (12)
    "#);
}

#[test]
fn test_git_clone_with_depth_subprocess() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.auto-local-bookmark = true");
    test_env.add_config("git.subprocess = true");
    let clone_path = test_env.env_root().join("clone");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git2::Repository::init(git_repo_path).unwrap();
    set_up_non_empty_git_repo(&git_repo);

    // git does support shallow clones on the local transport, so it will work
    // (we cannot replicate git2's erroneous behaviour wrt git)
    let (stdout, stderr) = test_env.jj_cmd_ok(
        test_env.env_root(),
        &["git", "clone", "--depth", "1", "source", "clone"],
    );
    insta::assert_snapshot!(stdout, @"");
    insta::assert_snapshot!(stderr, @r#"
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] tracked
    Setting the revset alias `trunk()` to `main@origin`
    Working copy now at: sqpuoqvx cad212e1 (empty) (no description set)
    Parent commit      : mzyxwzks 9f01a0e0 main | message
    Added 1 files, modified 0 files, removed 0 files
    "#);

    let (stdout, stderr) = test_env.jj_cmd_ok(&clone_path, &["log"]);
    insta::assert_snapshot!(stdout, @r"
    @  sqpuoqvx test.user@example.com 2001-02-03 08:05:07 cad212e1
    │  (empty) (no description set)
    ◆  mzyxwzks some.one@example.com 1970-01-01 11:00:00 main 9f01a0e0
    │  message
    ~
    ");
    insta::assert_snapshot!(stderr, @"");
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_invalid_immutable_heads(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if subprocess {
        test_env.add_config("git.subprocess = true");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git2::Repository::init(git_repo_path).unwrap();
    set_up_non_empty_git_repo(&git_repo);

    test_env.add_config("revset-aliases.'immutable_heads()' = 'unknown'");
    // Suppress lengthy warnings in commit summary template
    test_env.add_config("revsets.short-prefixes = ''");

    // The error shouldn't be counted as an immutable working-copy commit. It
    // should be reported.
    let stderr = test_env.jj_cmd_failure(test_env.env_root(), &["git", "clone", "source", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r#"
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] untracked
    Config error: Invalid `revset-aliases.immutable_heads()`
    Caused by: Revision `unknown` doesn't exist
    For help, see https://jj-vcs.github.io/jj/latest/config/.
    "#);
    }
}

#[test_case(false; "use git2 for remote calls")]
#[test_case(true; "spawn a git subprocess for remote calls")]
fn test_git_clone_malformed(subprocess: bool) {
    let test_env = TestEnvironment::default();
    if subprocess {
        test_env.add_config("git.subprocess = true");
    }
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git2::Repository::init(git_repo_path).unwrap();
    let clone_path = test_env.env_root().join("clone");
    // libgit2 doesn't allow to create a malformed repo containing ".git", etc.,
    // but we can insert ".jj" entry.
    set_up_git_repo_with_file(&git_repo, ".jj");

    // TODO: Perhaps, this should be a user error, not an internal error.
    let stderr =
        test_env.jj_cmd_internal_error(test_env.env_root(), &["git", "clone", "source", "clone"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r#"
    Fetching into new repo in "$TEST_ENV/clone"
    bookmark: main@origin [new] untracked
    Setting the revset alias `trunk()` to `main@origin`
    Internal error: Failed to check out commit 039a1eae03465fd3be0fbad87c9ca97303742677
    Caused by: Reserved path component .jj in $TEST_ENV/clone/.jj
    "#);
    }

    // The cloned workspace isn't usable.
    let stderr = test_env.jj_cmd_failure(&clone_path, &["status"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r##"
    Error: The working copy is stale (not updated since operation 4a8ddda0ff63).
    Hint: Run `jj workspace update-stale` to update it.
    See https://jj-vcs.github.io/jj/latest/working-copy/#stale-working-copy for more information.
    "##);
    }

    // The error can be somehow recovered.
    // TODO: add an update-stale flag to reset the working-copy?
    let stderr = test_env.jj_cmd_internal_error(&clone_path, &["workspace", "update-stale"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @r#"
    Internal error: Failed to check out commit 039a1eae03465fd3be0fbad87c9ca97303742677
    Caused by: Reserved path component .jj in $TEST_ENV/clone/.jj
    "#);
    }
    let (_stdout, stderr) =
        test_env.jj_cmd_ok(&clone_path, &["new", "root()", "--ignore-working-copy"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stderr, @"");
    }
    let stdout = test_env.jj_cmd_success(&clone_path, &["status"]);
    insta::allow_duplicates! {
    insta::assert_snapshot!(stdout, @r#"
    The working copy has no changes.
    Working copy : zsuskuln f652c321 (empty) (no description set)
    Parent commit: zzzzzzzz 00000000 (empty) (no description set)
    "#);
    }
}

#[test]
fn test_git_clone_no_git_executable() {
    let test_env = TestEnvironment::default();
    test_env.add_config("git.subprocess = true");
    test_env.add_config("git.executable-path = 'jj-test-missing-program'");
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git2::Repository::init(git_repo_path).unwrap();
    set_up_non_empty_git_repo(&git_repo);

    let stderr = test_env.jj_cmd_failure(test_env.env_root(), &["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(strip_last_line(&stderr), @r#"
    Fetching into new repo in "$TEST_ENV/clone"
    Error: Could not execute the git process, found in the OS path 'jj-test-missing-program'
    "#);
}

#[test]
fn test_git_clone_no_git_executable_with_path() {
    let test_env = TestEnvironment::default();
    let invalid_git_executable_path = test_env.env_root().join("invalid").join("path");
    test_env.add_config("git.subprocess = true");
    test_env.add_config(format!(
        "git.executable-path = {}",
        to_toml_value(invalid_git_executable_path.to_str().unwrap())
    ));
    let git_repo_path = test_env.env_root().join("source");
    let git_repo = git2::Repository::init(git_repo_path).unwrap();
    set_up_non_empty_git_repo(&git_repo);

    let stderr = test_env.jj_cmd_failure(test_env.env_root(), &["git", "clone", "source", "clone"]);
    insta::assert_snapshot!(strip_last_line(&stderr), @r#"
    Fetching into new repo in "$TEST_ENV/clone"
    Error: Could not execute git process at specified path '$TEST_ENV/invalid/path'
    "#);
}

fn get_bookmark_output(test_env: &TestEnvironment, repo_path: &Path) -> String {
    test_env.jj_cmd_success(repo_path, &["bookmark", "list", "--all-remotes"])
}
