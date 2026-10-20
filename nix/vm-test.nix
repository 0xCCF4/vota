# NixOS VM test: boots a machine with `services.vota` and checks the
# deployed service end to end. Run with
#   nix build .#checks.x86_64-linux.nixos-vm -L
# (or `nix flake check`). It also runs without KVM, emulated and much slower.
{ self, pkgs }:
let
  inherit (pkgs) lib;
  migrations = lib.length (
    lib.filter (lib.hasSuffix ".sql") (lib.attrNames (builtins.readDir ../migrations))
  );
  logos = pkgs.writeTextDir "acme.svg" ''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#c00"/></svg>'';
in
pkgs.testers.runNixOSTest {
  name = "vota";

  # KVM is used when the builder has it; without, QEMU emulates (slowly).
  requiredFeatures.kvm = false;

  nodes.machine = {
    imports = [ self.nixosModules.default ];

    services.vota = {
      enable = true;
      # Short, so the test sees a restart soon.
      healthCheck.interval = "5s";
      settings = {
        # Under a sub-path, as behind a shared reverse proxy.
        public_base_url = "http://localhost:8080/polls/";
        default_time_zone = "Europe/Berlin";
        trusted_proxies = [
          "127.0.0.1/32"
          "::1/128"
        ];
        logo_folder = logos;
        purge_interval = "15m";
        rate_limit = {
          failed_lookups = 5;
          admin_failed_auth = 3;
        };
        connections = {
          max_open = 64;
          header_read_timeout = "3s";
          shutdown_grace_period = "2s";
        };
      };
    };

    environment.systemPackages = [
      pkgs.curl
      pkgs.jq
      pkgs.sqlite
      pkgs.python3
    ];
  };

  testScript = ''
    import json

    def journal():
        return machine.succeed("journalctl -u vota.service -o cat --no-pager")

    def create_ballot(token):
        deadline = machine.succeed("date -u -d '+7 days' +%Y-%m-%dT%H:%M:%SZ").strip()
        body = json.dumps({
            "title": "Lunch on Friday?",
            "options": [{"label": "Pizza"}, {"label": "Sushi"}],
            "voting_deadline": deadline,
            "logo": "acme",
        })
        machine.succeed(f"echo '{body}' > /tmp/ballot.json")
        status = machine.succeed(
            "curl -s -o /tmp/created.json -w '%{http_code}' -X POST "
            "http://127.0.0.1:8080/polls/admin/ballots "
            f"-H 'Authorization: Bearer {token}' -H 'Content-Type: application/json' "
            "--data @/tmp/ballot.json"
        )
        assert status == "201", status
        return json.loads(machine.succeed("cat /tmp/created.json"))

    machine.wait_for_unit("vota.service")
    machine.wait_for_open_port(8080)

    with subtest("the service applied its migrations and answers /healthz"):
        health = json.loads(machine.succeed("curl -sf http://127.0.0.1:8080/healthz"))
        assert health["status"] == "ok", health
        # Local, but passed on by a proxy: the plain 404 of an unknown path.
        for header in ["X-Forwarded-For: 192.0.2.1", "Forwarded: for=192.0.2.1"]:
            status = machine.succeed(
                "curl -s -o /dev/null -w '%{http_code}' "
                f"-H '{header}' http://127.0.0.1:8080/polls/healthz"
            )
            assert status == "404", (header, status)
        applied = machine.succeed(
            "sqlite3 -readonly /var/lib/vota/vota.db "
            "'SELECT COUNT(*) FROM _sqlx_migrations WHERE success'"
        ).strip()
        assert applied == "${toString migrations}", applied
        assert "logos: acme" in journal()

    with subtest("an Admin Token from the CLI wrapper works for POST /admin/ballots"):
        token = machine.succeed("vota-admin-token create --label vm-test").strip()
        assert token.startswith("vta_") and len(token) == 47, token
        listed = json.loads(machine.succeed("vota-admin-token list --json"))
        assert [t["label"] for t in listed] == ["vm-test"], listed
        assert token not in journal()
        machine.fail("vota-admin-token revoke 99")
        created = create_ballot(token)

    with subtest("vota ballot creates and lists Ballots against the service"):
        # The client is on the PATH; the token comes from the wrapper.
        deadline = machine.succeed("date -u -d '+7 days' +%Y-%m-%dT%H:%M:%SZ").strip()
        client = f"VOTA_ADMIN_TOKEN={token} VOTA_URL=http://localhost:8080/polls vota ballot"
        url = machine.succeed(
            f"{client} create --title 'Lunch on Friday?' --option Pizza --option Sushi "
            f"--voting-deadline {deadline} --group vm-client --voter-reference alice"
        ).strip()
        assert url.startswith("http://localhost:8080/polls/b/"), url
        listed = json.loads(machine.succeed(f"{client} list --group vm-client --json"))
        assert [(b["url"], b["voter_reference"]) for b in listed["ballots"]] == [(url, "alice")], listed
        print(machine.succeed(f"{client} list --group vm-client"))
        assert token not in journal()
        # Without a token: a usage error, before any request.
        status = machine.execute("VOTA_URL=http://localhost:8080/polls vota ballot list")[0]
        assert status == 2, status
        # The shell completions are installed with the package.
        machine.succeed("grep -q 'vota' /run/current-system/sw/share/bash-completion/completions/vota.bash")

    with subtest("the CLI wrapper lists and revokes Admin Tokens"):
        # The commands of the deployment guide; their output is printed for it.
        print(machine.succeed(
            "vota-admin-token create --label 'survey scripts' --expires-in 180days 2>&1"
        ))
        print(machine.succeed("vota-admin-token list"))
        revoked = machine.succeed("vota-admin-token revoke --label 'survey scripts' 2>&1")
        assert revoked.strip() == "Revoked Admin Tokens 2.", revoked

    with subtest("the CLI wrapper passes $VAR in arguments literally"):
        machine.succeed("vota-admin-token create --label 'Bob $HOME ''${HOME} $UNSET'")
        listed = json.loads(machine.succeed("vota-admin-token list --json"))
        labels = [t["label"] for t in listed]
        assert "Bob $HOME ''${HOME} $UNSET" in labels, labels
        revoked = machine.succeed("vota-admin-token revoke --label 'Bob $HOME ''${HOME} $UNSET' 2>&1")
        assert revoked.startswith("Revoked Admin Token"), revoked
        listed = json.loads(machine.succeed("vota-admin-token list --json"))
        assert [t["label"] for t in listed] == ["vm-test"], listed

    with subtest("the returned Ballot URL is reachable and the voter API returns the Ballot"):
        url = created["url"]
        assert url == "http://localhost:8080/polls/b/" + created["token"], url
        page = machine.succeed(f"curl -sf {url}")
        assert "Lunch on Friday?" in page
        ballot = json.loads(
            machine.succeed(f"curl -sf http://localhost:8080/polls/api/b/{created['token']}")
        )
        assert ballot["title"] == "Lunch on Friday?", ballot
        assert ballot["status"] == "unanswered", ballot
        assert ballot["logo_url"] == "http://localhost:8080/polls/assets/logos/acme", ballot
        machine.succeed("curl -sf http://localhost:8080/polls/assets/logos/acme | grep -q '<svg'")

    with subtest("hardening is in effect"):
        assert machine.succeed("systemctl show -p DynamicUser --value vota").strip() == "yes"
        pid = machine.succeed("systemctl show -p MainPID --value vota").strip()
        uid = machine.succeed(f"stat -c %u /proc/{pid}").strip()
        assert uid != "0", uid
        assert machine.succeed(f"ps -o user= -p {pid}").strip() == "vota"
        # Commands in the service's namespaces, as its user.
        inside = f"nsenter --target {pid} --mount --user --setuid {uid} --setgid {uid} --"
        machine.succeed(f"{inside} touch /var/lib/vota/write-probe")
        machine.succeed(f"{inside} rm /var/lib/vota/write-probe")
        for path in ["/etc", "/var/lib", "/var/log", "/run", "/root", "/home", "/nix/store"]:
            machine.fail(f"{inside} touch {path}/write-probe")
        print(machine.succeed("systemd-analyze security vota.service | tail -n 1"))

    with subtest("connection limits are in effect"):
        assert machine.succeed("systemctl show -p LimitNOFILE --value vota").strip() == "576"
        # A client that sends nothing is disconnected after header_read_timeout.
        machine.succeed(
            "python3 -c 'import socket,time; s=socket.create_connection((\"127.0.0.1\",8080)); "
            "s.settimeout(30); t=time.time(); assert s.recv(10)==b\"\"; assert time.time()-t<10'"
        )

    with subtest("systemctl stop shuts the service down gracefully"):
        machine.succeed("systemctl stop vota.service")
        log = journal()
        print(log)
        assert "received SIGTERM, shutting down gracefully" in log, log
        assert "purge sweep stopped; shut down" in log, log
        for bad in ["ERROR", "WARN", "panicked", "Failed"]:
            assert bad not in log, bad
        show = machine.succeed(
            "systemctl show -p Result -p ExecMainCode -p ExecMainStatus vota"
        )
        # ExecMainCode=1 is CLD_EXITED: the process exited, not killed by a signal.
        assert "Result=success" in show and "ExecMainCode=1" in show, show
        assert "ExecMainStatus=0" in show, show

    with subtest("state survives a restart under a new dynamic user"):
        listed = json.loads(machine.succeed("vota-admin-token list --json"))
        assert [t["label"] for t in listed] == ["vm-test"], listed
        machine.succeed("systemctl start vota.service")
        machine.wait_for_open_port(8080)
        create_ballot(token)

    with subtest("admin requests are audited and failed Admin Token checks rate-limited"):
        log = journal()
        assert "admin request token_id=1 method=POST route=/polls/admin/ballots status=201" in log, log
        assert token not in log
        listed = json.loads(machine.succeed("vota-admin-token list --json"))
        assert listed[0]["last_used_at"] is not None, listed
        # A client behind the (trusted) local proxy, with a wrong token.
        statuses = [
            machine.succeed(
                "curl -s -o /dev/null -w '%{http_code}' -H 'X-Forwarded-For: 203.0.113.9' "
                "-H 'Authorization: Bearer vta_wrong' http://127.0.0.1:8080/polls/admin/ballots"
            )
            for _ in range(4)
        ]
        assert statuses == ["401", "401", "401", "429"], statuses
        log = journal()
        assert "too many failed Admin Token checks" in log and "client_ip=203.0.113.9" in log, log
        assert "vta_wrong" not in log
        # Other clients are not affected.
        create_ballot(token)

    def main_pid():
        return machine.succeed("systemctl show -p MainPID --value vota").strip()

    with subtest("startup waits for /healthz under the base path, and the check passes"):
        post = machine.succeed("systemctl show -p ExecStartPost --value vota")
        assert "vota-wait-until-healthy" in post, post
        script = machine.succeed(
            "cat $(systemctl show -p ExecStart --value vota-health-check "
            "| grep -o '/nix/store/[^ ;]*')"
        )
        assert "http://127.0.0.1:8080/polls/healthz" in script, script
        machine.succeed("systemctl is-active vota-health-check.timer")
        pid = main_pid()
        machine.succeed("systemctl start vota-health-check.service")
        assert main_pid() == pid
        checks = machine.succeed("journalctl -u vota-health-check.service -o cat --no-pager")
        assert "restarting" not in checks, checks

    with subtest("a service that stops answering /healthz is restarted"):
        pid = main_pid()
        # Frozen: the port stays open, but nothing answers.
        machine.succeed(f"kill -STOP {pid}")
        machine.fail("curl -sf --max-time 3 http://127.0.0.1:8080/healthz")
        machine.wait_until_succeeds(
            f"[ \"$(systemctl show -p MainPID --value vota)\" != {pid} ] "
            "&& systemctl is-active vota.service "
            "&& curl -sf http://127.0.0.1:8080/polls/healthz",
            timeout=300,
        )
        checks = machine.succeed("journalctl -u vota.service -u vota-health-check.service -o cat --no-pager")
        print(checks)
        assert "restarting vota.service" in checks, checks
        create_ballot(token)

    with subtest("a database file the service can't make 0600 is logged, and it still starts"):
        db = "/var/lib/vota/vota.db"
        machine.succeed("systemctl stop vota.service")
        # systemd keeps the dynamic user's state directory owned by nobody
        # and ID-maps nobody to the service's user (and root to root), so a
        # file owned by nobody is the service's own. One owned by root is
        # not: the service can't change its mode (EPERM), but writes it
        # through mode 0666. The directory keeps its owner, so systemd
        # doesn't chown the files in it back on start.
        machine.succeed(f"chown root:root {db} && chmod 0666 {db}")
        machine.succeed("systemctl start vota.service")
        machine.wait_for_open_port(8080)
        log = journal()
        print(log)
        warning = f"could not make {db} readable by its owner only (mode 0600); other users may be able to read it"
        assert warning in log, log
        assert machine.succeed(f"stat -c '%U %a' {db}").strip() == "root 666"
        create_ballot(token)
        # Back to the service's user: the next start makes the file 0600.
        machine.succeed("systemctl stop vota.service")
        machine.succeed(f"chown --reference=/var/lib/private/vota {db}")
        machine.succeed("systemctl start vota.service")
        machine.wait_for_open_port(8080)
        assert machine.succeed(f"stat -c %a {db}").strip() == "600"
        assert journal().count(warning) == 1
        create_ballot(token)
  '';
}
