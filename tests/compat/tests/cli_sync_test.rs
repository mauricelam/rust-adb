use compat::{normalize_output, GoldenCase, TestOutput, TestServer};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

fn get_mock_adbd_bin() -> PathBuf {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = PathBuf::from(manifest_dir).join("../..");
    let bin_name = if cfg!(target_os = "windows") {
        "mock-adbd.exe"
    } else {
        "mock-adbd"
    };
    let bin_path = root.join("target").join("debug").join(bin_name);
    if !bin_path.exists() {
        let status = Command::new("cargo")
            .args(["build", "-p", "mock-adbd"])
            .status()
            .expect("Failed to build mock-adbd binary");
        assert!(status.success(), "Failed to build mock-adbd");
    }
    bin_path
}

struct MockAdbdProcess {
    child: Child,
    pub addr: String,
}

impl MockAdbdProcess {
    fn start_with_args(extra_args: &[&str]) -> Self {
        let bin = get_mock_adbd_bin();
        let mut cmd = Command::new(&bin);
        cmd.arg("--listen").arg("127.0.0.1:0");
        for arg in extra_args {
            cmd.arg(arg);
        }
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|e| panic!("Failed to spawn mock-adbd binary at {:?}: {}", bin, e));

        let stdout = child.stdout.take().expect("Failed to open stdout");
        let mut reader = BufReader::new(stdout);
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .expect("Failed to read first line from mock-adbd stdout");
        let addr = first_line.trim().to_string();

        MockAdbdProcess { child, addr }
    }
}

impl Drop for MockAdbdProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn load_golden(case_name: &str) -> GoldenCase {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let golden_path = PathBuf::from(manifest_dir)
        .join("golden")
        .join(format!("{case_name}.json"));
    let content = fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("Failed to read golden file {}: {e}", golden_path.display()));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("Failed to parse golden file {}: {e}", golden_path.display()))
}

fn assert_matches_golden(case_name: &str, actual: &TestOutput) {
    let golden = load_golden(case_name);
    let norm_stdout = normalize_output(&actual.stdout);
    let norm_stderr = normalize_output(&actual.stderr);

    assert_eq!(
        norm_stdout, golden.stdout,
        "stdout mismatch for golden test case '{case_name}'"
    );
    assert_eq!(
        norm_stderr, golden.stderr,
        "stderr mismatch for golden test case '{case_name}'"
    );
    assert_eq!(
        actual.exit_code, golden.exit_code,
        "exit_code mismatch for golden test case '{case_name}'"
    );
}

fn run_sync_test_matrix(v2_args: &[&str], prefix: &str) {
    let mock = MockAdbdProcess::start_with_args(v2_args);
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(connect_out.success());

    // 1. adb ls /data
    let ls_out = server.run_client(&["-s", &mock.addr, "ls", "/data"]);
    assert_matches_golden(
        &format!(
            "mock_sync_ls_data_{}",
            if prefix == "sync_v2" { "v2" } else { "v1" }
        ),
        &ls_out,
    );

    // 2. adb pull and push for 0, 1, 65535, 65536, 300000 bytes files
    let tmp_dir = tempfile::tempdir().unwrap();

    let test_files = [
        ("f_0_bin", "f_0.bin", vec![]),
        ("f_1_bin", "f_1.bin", vec![0x42]),
        ("f_65535_bin", "f_65535.bin", vec![0xAA; 65535]),
        ("f_65536_bin", "f_65536.bin", vec![0xBB; 65536]),
        (
            "f_300k_bin",
            "f_300k.bin",
            (0..307200).map(|i| (i % 256) as u8).collect(),
        ),
    ];

    for (file_key, filename, expected_bytes) in &test_files {
        let remote_path = format!("/data/{filename}");
        let local_path = tmp_dir.path().join(filename);

        let pull_out = server.run_client(&[
            "-s",
            &mock.addr,
            "pull",
            &remote_path,
            local_path.to_str().unwrap(),
        ]);
        assert_matches_golden(&format!("{prefix}_pull_{file_key}"), &pull_out);

        let pulled_bytes = std::fs::read(&local_path).unwrap();
        assert_eq!(
            &pulled_bytes, expected_bytes,
            "Pulled bytes mismatch for {filename}"
        );

        // Push
        let pushed_filename = format!("pushed_{filename}");
        let local_src = tmp_dir.path().join(&pushed_filename);
        std::fs::write(&local_src, expected_bytes).unwrap();

        let remote_dest = format!("/data/{pushed_filename}");
        let push_out = server.run_client(&[
            "-s",
            &mock.addr,
            "push",
            local_src.to_str().unwrap(),
            &remote_dest,
        ]);
        assert_matches_golden(&format!("{prefix}_push_{file_key}"), &push_out);

        let verify_local = tmp_dir.path().join(format!("verify_{pushed_filename}"));
        let re_pull = server.run_client(&[
            "-s",
            &mock.addr,
            "pull",
            &remote_dest,
            verify_local.to_str().unwrap(),
        ]);
        assert!(re_pull.success());
        let re_pulled_bytes = std::fs::read(&verify_local).unwrap();
        assert_eq!(
            &re_pulled_bytes, expected_bytes,
            "Pushed & re-pulled bytes mismatch for {filename}"
        );
    }

    // 3. adb pull -a
    let pull_a_local = tmp_dir.path().join("pull_a.bin");
    let pull_a_out = server.run_client(&[
        "-s",
        &mock.addr,
        "pull",
        "-a",
        "/data/f_1.bin",
        pull_a_local.to_str().unwrap(),
    ]);
    assert_matches_golden(&format!("{prefix}_pull_a"), &pull_a_out);
    assert_eq!(std::fs::read(&pull_a_local).unwrap(), vec![0x42]);

    // 4. Missing remote file
    let missing_remote_local = tmp_dir.path().join("missing_remote.bin");
    let missing_pull_out = server.run_client(&[
        "-s",
        &mock.addr,
        "pull",
        "/data/nonexistent.bin",
        missing_remote_local.to_str().unwrap(),
    ]);
    assert_matches_golden(&format!("{prefix}_pull_missing_remote"), &missing_pull_out);

    // 5. Missing local file push
    let missing_local_out = server.run_client(&[
        "-s",
        &mock.addr,
        "push",
        tmp_dir.path().join("nosuchlocal.bin").to_str().unwrap(),
        "/data/nosuchlocal.bin",
    ]);
    assert_matches_golden(&format!("{prefix}_push_missing_local"), &missing_local_out);

    // 6. Destination directory does not exist for pull
    let bad_dst_pull_out = server.run_client(&[
        "-s",
        &mock.addr,
        "pull",
        "/data/f_1.bin",
        tmp_dir
            .path()
            .join("nonexistent_dir")
            .join("file.bin")
            .to_str()
            .unwrap(),
    ]);
    assert_matches_golden(&format!("{prefix}_pull_bad_dst_dir"), &bad_dst_pull_out);

    // 7. Two sources into a non-directory file
    let two_src_file = tmp_dir.path().join("single_target.bin");
    std::fs::write(&two_src_file, b"data").unwrap();
    let two_src_pull_out = server.run_client(&[
        "-s",
        &mock.addr,
        "pull",
        "/data/f_0.bin",
        "/data/f_1.bin",
        two_src_file.to_str().unwrap(),
    ]);
    assert_matches_golden(
        &format!("{prefix}_pull_two_sources_into_file"),
        &two_src_pull_out,
    );

    let src1 = tmp_dir.path().join("src1.bin");
    let src2 = tmp_dir.path().join("src2.bin");
    std::fs::write(&src1, b"1").unwrap();
    std::fs::write(&src2, b"2").unwrap();
    let two_src_push_out = server.run_client(&[
        "-s",
        &mock.addr,
        "push",
        src1.to_str().unwrap(),
        src2.to_str().unwrap(),
        "/data/f_0.bin",
    ]);
    assert_matches_golden(
        &format!("{prefix}_push_two_sources_into_file"),
        &two_src_push_out,
    );

    // 8. Push failure on device (/data/fail.bin triggers FAIL in mock-adbd)
    let push_fail_src = tmp_dir.path().join("push_fail.bin");
    std::fs::write(&push_fail_src, b"fail content").unwrap();
    let push_fail_out = server.run_client(&[
        "-s",
        &mock.addr,
        "push",
        push_fail_src.to_str().unwrap(),
        "/data/fail.bin",
    ]);
    assert_matches_golden(&format!("{prefix}_push_fail_device"), &push_fail_out);
}

#[test]
fn test_cli_sync_v2() {
    run_sync_test_matrix(&[], "sync_v2");
}

#[test]
fn test_cli_sync_v1() {
    run_sync_test_matrix(
        &["--no-stat-v2", "--no-ls-v2", "--no-sendrecv-v2"],
        "sync_v1",
    );
}
