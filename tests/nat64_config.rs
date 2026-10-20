//! `nat64_prefixes` is read from the TOML file or the environment
//! (`VOTA_NAT64_PREFIXES`); an entry that is not an RFC 6052 prefix
//! stops startup with a message saying why. Driven through the real binary,
//! as an operator would.

#![cfg(feature = "server")]

use std::path::Path;

use assert_cmd::Command;

/// `vota admin-token list` (any command loads the config) on a temp
/// database.
fn vota(dir: &Path) -> Command {
    let mut command = Command::cargo_bin("vota").unwrap();
    command
        .env_clear()
        .env("VOTA_DATABASE_PATH", dir.join("vota.db"))
        .args(["admin-token", "list"]);
    command
}

#[test]
fn nat64_prefixes_are_read_from_the_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("vota.toml");
    std::fs::write(
        &file,
        "nat64_prefixes = [\"2001:db8:64::/96\", \"2001:db8:46::/48\", \"2001:db8::/32\"]\n",
    )
    .unwrap();

    let output = vota(dir.path())
        .arg("--config")
        .arg(&file)
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
}

#[test]
fn nat64_prefixes_are_read_from_the_environment() {
    let dir = tempfile::tempdir().unwrap();

    let output = vota(dir.path())
        .env(
            "VOTA_NAT64_PREFIXES",
            r#"["2001:db8:64:1::/64", "2001:db8:100::/40", "2001:db8:122:300::/56"]"#,
        )
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("ignoring"), "{stderr}");
}

#[test]
fn unusable_nat64_prefixes_stop_startup_with_the_reason() {
    for (value, reason) in [
        (
            r#"["64:ff9b::/80"]"#,
            "a NAT64 prefix must have a prefix length of 32, 40, 48, 56, 64 or 96 \
             (RFC 6052): \"64:ff9b::/80\"",
        ),
        (
            r#"["2001:db8::/128"]"#,
            "must have a prefix length of 32, 40, 48, 56, 64 or 96",
        ),
        (
            r#"["192.0.2.0/24"]"#,
            "a NAT64 prefix must be an IPv6 prefix, not an IPv4 network: \"192.0.2.0/24\"",
        ),
        (
            r#"["::ffff:192.0.2.0/120"]"#,
            "must have a prefix length of 32, 40, 48, 56, 64 or 96",
        ),
        (
            r#"["64:ff9b::1/96"]"#,
            "a NAT64 prefix must not have host bits set: \"64:ff9b::1/96\" \
             (the prefix is 64:ff9b::/96)",
        ),
        (
            r#"["2001:db8:0:0:ff00::/96"]"#,
            "a NAT64 prefix must have bits 64 to 71 (the u-octet) set to zero \
             (RFC 6052): \"2001:db8:0:0:ff00::/96\"",
        ),
        (r#"["2001:db8:0:0:100::/96"]"#, "the u-octet"),
        (
            r#"["64:ff9b::"]"#,
            "a NAT64 prefix must be an IPv6 prefix in CIDR notation, such as \
             \"64:ff9b:1:2::/64\": \"64:ff9b::\"",
        ),
        (r#"["pool6"]"#, "in CIDR notation"),
        (r#""64:ff9b::/96""#, "expected a sequence"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let output = vota(dir.path())
            .env("VOTA_NAT64_PREFIXES", value)
            .output()
            .unwrap();

        assert!(!output.status.success(), "{value} was accepted");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(reason), "{value}: {stderr}");
        assert!(stderr.contains("NAT64_PREFIXES"), "{value}: {stderr}");
    }
}

#[test]
fn an_unusable_nat64_prefix_in_the_config_file_is_named_by_its_key() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("t.toml");
    std::fs::write(
        &file,
        "nat64_prefixes = [\"64:ff9b::/96\", \"10.0.0.0/8\"]\n",
    )
    .unwrap();

    let output = vota(dir.path())
        .arg("--config")
        .arg(&file)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains(
            "a NAT64 prefix must be an IPv6 prefix, not an IPv4 network: \"10.0.0.0/8\" \
             for key \"default.nat64_prefixes.1\""
        ),
        "{stderr}"
    );
}
