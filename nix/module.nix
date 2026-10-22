# NixOS module `services.vota`: Vota as a hardened systemd service,
# plus `vota-admin-token` on the system path to manage Admin Tokens in the
# service's database as the service's (dynamic) user, and `vota` itself
# for the admin API client `vota ballot …`.
{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib)
    mkEnableOption
    mkIf
    mkOption
    mkPackageOption
    types
    ;

  cfg = config.services.vota;
  stateDir = "/var/lib/vota";

  settingsFormat = pkgs.formats.toml { };
  # TOML has no null: an unset optional key is left out, so the app's own
  # default applies. The file holds no secrets, so it can live in the store.
  configFile = settingsFormat.generate "vota.toml" (
    lib.filterAttrsRecursive (_: value: value != null) cfg.settings
  );

  readOnlyFolders = lib.filter (folder: folder != null) [
    cfg.settings.theme_folder
    cfg.settings.logo_folder
  ];
  lowPort = cfg.settings.port < 1024;

  # Admin Token files reach the service as systemd credentials: systemd
  # (as root) copies each into the service's credentials directory, readable
  # by its dynamic user, so a root-only secret (agenix) needs no looser
  # permissions. The n-th file (from 1) is the credential
  # `vota-admin-token-<n>`, and the audit log's `token_id=file:<n>`.
  adminTokenCredentials = lib.imap1 (n: path: {
    name = "vota-admin-token-${toString n}";
    inherit path;
  }) cfg.adminTokenFiles;
  # `%d` is the credentials directory (a systemd specifier). Unquoted list
  # entries are fine: the names hold no `,`, `[`, `]` or spaces.
  adminTokenFilesEnv = "VOTA_ADMIN_TOKEN_FILES=[${
    lib.concatMapStringsSep "," (credential: "%d/${credential.name}") adminTokenCredentials
  }]";

  # Shared by the service and the admin-token CLI's transient unit, so both
  # run as the same dynamic user in the same sandbox.
  sandbox = {
    DynamicUser = true;
    User = "vota";
    Group = "vota";
    StateDirectory = "vota";
    StateDirectoryMode = "0700";
    WorkingDirectory = stateDir;
    UMask = "0077";
    Environment = [ "NO_COLOR=1" ];

    # Readable folders the app needs besides the store ("-": a missing one is
    # reported by the app itself, naming the option, not by systemd).
    # (Interpolation, like the TOML file, puts a Nix path into the store.)
    ReadOnlyPaths = map (folder: "-${folder}") readOnlyFolders;

    CapabilityBoundingSet = if lowPort then [ "CAP_NET_BIND_SERVICE" ] else [ "" ];
    AmbientCapabilities = if lowPort then [ "CAP_NET_BIND_SERVICE" ] else [ "" ];
    NoNewPrivileges = true;
    LockPersonality = true;
    MemoryDenyWriteExecute = true;
    PrivateDevices = true;
    PrivateTmp = true;
    # A user namespace would take CAP_NET_BIND_SERVICE away again.
    PrivateUsers = !lowPort;
    ProcSubset = "pid";
    ProtectClock = true;
    ProtectControlGroups = true;
    ProtectHome = true;
    ProtectHostname = true;
    ProtectKernelLogs = true;
    ProtectKernelModules = true;
    ProtectKernelTunables = true;
    ProtectProc = "invisible";
    ProtectSystem = "strict";
    RemoveIPC = true;
    RestrictAddressFamilies = [
      "AF_INET"
      "AF_INET6"
      "AF_UNIX"
    ];
    RestrictNamespaces = true;
    RestrictRealtime = true;
    RestrictSUIDSGID = true;
    SystemCallArchitectures = "native";
    SystemCallFilter = [
      "@system-service"
      "~@privileged @resources"
    ];
    SystemCallErrorNumber = "EPERM";
  };

  # `sandbox` as `systemd-run -p Key=Value` arguments: a list becomes one
  # argument per entry, as repeated lines would in a unit file.
  sandboxProperties = lib.concatLists (
    lib.mapAttrsToList (
      name: value:
      map (
        entry: "--property=${name}=${if lib.isBool entry then lib.boolToString entry else toString entry}"
      ) (lib.toList value)
    ) sandbox
  );

  adminTokenCli = pkgs.writeShellApplication {
    name = "vota-admin-token";
    runtimeInputs = [
      pkgs.coreutils
      config.systemd.package
    ];
    text = ''
      # Runs `vota admin-token "$@"` as the service's dynamic user against
      # its database (also while the service runs: SQLite is in WAL mode).
      # The new Admin Token is printed on this command's stdout.
      if [ "$(id -u)" -ne 0 ]; then
        echo "vota-admin-token: run this as root (e.g. with sudo)" >&2
        exit 1
      fi
      # --expand-environment=no: arguments such as a label with `$HOME` in it
      # reach the CLI literally, not expanded by the service manager.
      exec systemd-run --pipe --wait --quiet --collect --service-type=exec \
        --expand-environment=no \
        --description="Vota admin-token CLI" \
        ${lib.escapeShellArgs sandboxProperties} \
        -- ${lib.getExe cfg.package} --config ${configFile} admin-token "$@"
    '';
  };

  # The health check probes `/healthz` under the base path, from this host:
  # the app answers it only for a loopback TCP peer without forwarding
  # headers. A wildcard bind address is probed via loopback; a specific
  # non-loopback one can't be (the probe's peer would be that address).
  bindAddress = cfg.settings.bind_address;
  probeHost =
    if bindAddress == "0.0.0.0" then
      "127.0.0.1"
    else if bindAddress == "::" then
      "[::1]"
    else if lib.hasInfix ":" bindAddress then
      "[${bindAddress}]"
    else
      bindAddress;
  probeable =
    lib.elem bindAddress [
      "0.0.0.0"
      "::"
      "::1"
    ]
    || lib.hasPrefix "127." bindAddress;
  # The path of `public_base_url` with a trailing slash, as the app derives it.
  basePath =
    let
      matched = builtins.match "[^:]+://[^/?#]*(/[^?#]*)?.*" cfg.settings.public_base_url;
      path = if matched == null || lib.head matched == null then "/" else lib.head matched;
    in
    if lib.hasSuffix "/" path then path else "${path}/";
  healthUrl = "http://${probeHost}:${toString cfg.settings.port}${basePath}healthz";
  probe = "curl --silent --fail --noproxy '*' --output /dev/null --max-time 5 ${lib.escapeShellArg healthUrl}";

  # ExecStartPost: the service counts as started once `/healthz` answers.
  waitUntilHealthy = pkgs.writeShellApplication {
    name = "vota-wait-until-healthy";
    runtimeInputs = [ pkgs.curl ];
    text = ''
      deadline=$((SECONDS + 60))
      until ${probe}; do
        if [ "$SECONDS" -ge "$deadline" ]; then
          echo "vota: ${healthUrl} did not answer within 60 seconds" >&2
          exit 1
        fi
        sleep 0.5
      done
    '';
  };

  # The periodic check: restarts a running service whose `/healthz` stops
  # answering. A stopped service stays stopped.
  healthCheck = pkgs.writeShellApplication {
    name = "vota-health-check";
    runtimeInputs = [
      pkgs.curl
      config.systemd.package
    ];
    text = ''
      if [ "$(systemctl is-active vota.service)" != active ]; then
        exit 0
      fi
      for attempt in 1 2 3; do
        if ${probe}; then
          exit 0
        fi
        echo "vota: ${healthUrl} did not answer (attempt $attempt of 3)" >&2
        sleep 2
      done
      echo "vota: restarting vota.service" >&2
      # try-restart: a service stopped while the probes ran stays stopped.
      systemctl try-restart vota.service
    '';
  };

  duration =
    default: example:
    mkOption {
      type = types.str;
      inherit default example;
      description = "A duration in [humantime](https://docs.rs/humantime/latest/humantime/fn.parse_duration.html) syntax.";
    };
in
{
  options.services.vota = {
    enable = mkEnableOption "Vota, personal Ballots with an admin API and a voting UI";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.vota;
      defaultText = lib.literalExpression "vota.packages.\${pkgs.stdenv.hostPlatform.system}.vota";
      description = "The Vota package to run.";
    };

    adminTokenFiles = mkOption {
      type = types.listOf types.str;
      default = [ ];
      example = [ "/run/agenix/vota-admin-token" ];
      description = ''
        Files that each hold one pre-provisioned Admin Token (`vta_` and 43
        base64url characters, e.g. from `vota admin-token generate`), such
        as agenix secrets. The admin API accepts them beside the tokens of
        `vota-admin-token create`. Absolute paths outside the Nix store, as
        strings: a Nix path would copy the secret into the world-readable
        store. They are passed in with systemd's `LoadCredential`, so a
        root-owned `0400` file works. Read when the service starts: a
        missing or malformed file stops it; a changed file takes effect,
        and a removed entry stops its token working, at the next restart.
        Not listed or revoked by `vota-admin-token`.
      '';
    };

    openFirewall = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Whether to open `settings.port` in the firewall. Only useful with a
        non-loopback `settings.bind_address`, e.g. for a reverse proxy on
        another machine; a proxy on the same host needs no open port.
      '';
    };

    healthCheck = {
      enable = mkOption {
        type = types.bool;
        default = true;
        description = ''
          Whether the service checks itself through a local `/healthz`
          request: it counts as started only once `/healthz` answers
          (`ExecStartPost`, at most 60 seconds), and a timer restarts it when
          `/healthz` stops answering. The probe uses `settings.bind_address`
          (a wildcard address via loopback), `settings.port` and the base
          path of `settings.public_base_url`. `/healthz` answers only
          loopback peers, so this needs a loopback or wildcard bind address.
        '';
      };
      interval = mkOption {
        type = types.str;
        default = "1min";
        example = "30s";
        description = "How often the timer checks `/healthz` (a systemd time span).";
      };
    };

    settings = mkOption {
      description = ''
        Vota's configuration, written to a TOML file in the Nix store
        (it holds no secrets) and passed with `--config`. Keys and values are
        those of Vota's configuration reference
        (`docs/reference/configuration.md`); options left `null` are omitted,
        so the app's default applies. Any other key is passed through, and
        the app refuses to start on a key it doesn't know.
      '';
      default = { };
      type = types.submodule {
        freeformType = settingsFormat.type;
        options = {
          bind_address = mkOption {
            type = types.str;
            default = "127.0.0.1";
            example = "::";
            description = "IP address (no port) the single listener binds to.";
          };
          port = mkOption {
            type = types.port;
            default = 8080;
            description = "Port of the listener: voter UI, voter API, admin API and assets.";
          };
          public_base_url = mkOption {
            type = types.str;
            example = "https://example.org/polls/";
            description = ''
              The public URL Vota is reached at, the base of every Ballot
              URL. Its path is the base path every route is served under; the
              reverse proxy forwards that prefix unchanged.
            '';
          };
          database_path = mkOption {
            type = types.str;
            default = "${stateDir}/vota.db";
            description = ''
              The SQLite database file. It must lie directly in the service's
              StateDirectory `${stateDir}`, the only place its dynamic user
              can write to.
            '';
          };
          theme_folder = mkOption {
            type = types.nullOr types.path;
            default = null;
            example = lib.literalExpression "./themes";
            description = ''
              Folder with additional Themes. A Nix path is copied to the
              store; a string path must be readable by any user and not lie
              under /home, /root or /run/user (ProtectHome). If set, it must
              exist, or the service fails to start.
            '';
          };
          logo_folder = mkOption {
            type = types.nullOr types.path;
            default = null;
            example = "/etc/vota/logos";
            description = ''
              Folder with logos for the `logo` Theme Tweak. Same rules as
              `theme_folder`.
            '';
          };
          default_purge_gap = duration "4weeks" "2weeks" // {
            description = "Default gap between Voting Deadline and Purge Date (humantime duration).";
          };
          purge_interval = duration "1h" "15m" // {
            description = "How often the purge sweep runs (humantime duration, greater than zero).";
          };
          max_date_horizon = duration "1y" "2y" // {
            description = "How far ahead an admin may set a Voting Deadline or Purge Date (humantime duration, greater than zero).";
          };
          default_time_zone = mkOption {
            type = types.str;
            # The system's zone; `time.timeZone` is null when it is set
            # imperatively (timedatectl), and the app's own default is UTC.
            default = if config.time.timeZone != null then config.time.timeZone else "UTC";
            defaultText = lib.literalExpression ''if config.time.timeZone != null then config.time.timeZone else "UTC"'';
            example = "Europe/Berlin";
            description = ''
              IANA time zone for times shown on the voting page without
              JavaScript, and in which the admin API reads a Voting Deadline
              or Purge Date given without a UTC offset. Defaults to the
              system's `time.timeZone`, or `UTC` when that is unset.
            '';
          };
          trusted_proxies = mkOption {
            type = types.listOf types.str;
            default = [ ];
            example = [
              "127.0.0.1/32"
              "::1/128"
            ];
            description = "Networks (CIDR, with prefix length) whose `X-Forwarded-For` header is trusted.";
          };
          nat64_prefixes = mkOption {
            type = types.listOf (types.strMatching ".+/(32|40|48|56|64|96)");
            default = [ ];
            example = [ "2001:db8:64:1::/64" ];
            description = ''
              The NAT64/SIIT translator's IPv6 prefixes (e.g. Jool's `pool6`;
              RFC 6052 lengths 32, 40, 48, 56, 64 or 96, no host bits set, bits
              64-71 zero). A client address in one counts as the IPv4 address
              embedded in it for the rate limits and the audit log. `64:ff9b::/96` and
              `64:ff9b:1::/48` are always decoded.
            '';
          };
          rate_limit = {
            failed_lookups = mkOption {
              type = types.ints.between 1 4294967295;
              default = 20;
              description = "Failed Ballot Token lookups one client IP may make per window.";
            };
            window = duration "10m" "30m" // {
              description = "Length of the rate-limit window (humantime duration, greater than zero).";
            };
            admin_failed_auth = mkOption {
              type = types.ints.between 1 4294967295;
              default = 10;
              description = "Failed Admin Token checks one client IP may make per `admin_window` before its admin requests get 429.";
            };
            admin_window = duration "10m" "1h" // {
              description = "Length of the window of `admin_failed_auth` (humantime duration, greater than zero).";
            };
          };
          connections = {
            max_open = mkOption {
              type = types.ints.between 1 1000000;
              default = 512;
              description = ''
                How many connections the listener serves at once; further
                ones wait in the kernel's listen queue. The service's
                `LimitNOFILE` is set to this plus 512.
              '';
            };
            header_read_timeout = duration "10s" "30s" // {
              description = ''
                How long a client may take to send a request's headers, on a
                new connection and between requests on a kept-alive one (so
                also how long an idle connection stays open); humantime
                duration, greater than zero.
              '';
            };
            shutdown_grace_period = duration "10s" "30s" // {
              description = ''
                On stop: how long open connections get to finish before they
                are closed regardless (humantime duration). Keep it well
                below systemd's `TimeoutStopSec` (90 s by default).
              '';
            };
          };
        };
      };
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = dirOf cfg.settings.database_path == stateDir;
        message = "services.vota.settings.database_path must lie directly in ${stateDir}, the service's StateDirectory.";
      }
      {
        assertion = !(cfg.settings ? admin_token_files);
        message = "services.vota.settings.admin_token_files is not supported; use services.vota.adminTokenFiles, which passes the files in as systemd credentials.";
      }
    ]
    ++ map (path: {
      assertion =
        lib.hasPrefix "/" path
        && !lib.hasPrefix "${builtins.storeDir}/" path
        && builtins.match ".*[[:space:]].*" path == null;
      message = "services.vota.adminTokenFiles: ${path} must be an absolute path without whitespace, outside the Nix store (a secret there is world-readable).";
    }) cfg.adminTokenFiles
    ++ [
      {
        assertion = cfg.healthCheck.enable -> probeable;
        message = "services.vota.healthCheck needs a loopback or wildcard settings.bind_address (/healthz answers only loopback peers); set healthCheck.enable = false for ${bindAddress}.";
      }
    ];

    # The wrapper for Admin Tokens, and the package itself for the admin API
    # client `vota ballot …` (with its shell completions).
    environment.systemPackages = [
      adminTokenCli
      cfg.package
    ];

    networking.firewall.allowedTCPPorts = mkIf cfg.openFirewall [ cfg.settings.port ];

    systemd.services.vota = {
      description = "Vota";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      # Restart on a configuration change, e.g. a new Theme folder.
      restartTriggers = [ configFile ];
      serviceConfig = sandbox // {
        ExecStart = "${lib.getExe cfg.package} --config ${configFile} serve";
        # The `%` of a path is escaped: systemd would read it as a specifier.
        LoadCredential = map (
          credential: "${credential.name}:${lib.replaceStrings [ "%" ] [ "%%" ] credential.path}"
        ) adminTokenCredentials;
        Environment = sandbox.Environment ++ lib.optional (adminTokenCredentials != [ ]) adminTokenFilesEnv;
        # Default KillSignal (SIGTERM): the app shuts down gracefully on it,
        # within `settings.connections.shutdown_grace_period`.
        Restart = "on-failure";
        # A deliberate descriptor limit: the open connections plus room for
        # the database, the listener and the runtime.
        LimitNOFILE = cfg.settings.connections.max_open + 512;
        ExecStartPost = mkIf cfg.healthCheck.enable (lib.getExe waitUntilHealthy);
      };
    };

    # Started and stopped with the service (PartOf), so a stopped service
    # isn't brought back.
    systemd.timers.vota-health-check = mkIf cfg.healthCheck.enable {
      description = "Periodic Vota health check";
      wantedBy = [ "vota.service" ];
      partOf = [ "vota.service" ];
      timerConfig = {
        OnActiveSec = cfg.healthCheck.interval;
        OnUnitActiveSec = cfg.healthCheck.interval;
        AccuracySec = "1s";
      };
    };

    # Root, only to restart the service; otherwise as locked down as it gets
    # with `systemctl` (AF_UNIX to systemd) and a loopback HTTP request.
    systemd.services.vota-health-check = mkIf cfg.healthCheck.enable {
      description = "Vota health check";
      serviceConfig = {
        Type = "oneshot";
        ExecStart = lib.getExe healthCheck;
        CapabilityBoundingSet = [ "" ];
        NoNewPrivileges = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        PrivateDevices = true;
        PrivateTmp = true;
        ProtectClock = true;
        ProtectControlGroups = true;
        ProtectHome = true;
        ProtectHostname = true;
        ProtectKernelLogs = true;
        ProtectKernelModules = true;
        ProtectKernelTunables = true;
        ProtectSystem = "strict";
        RestrictAddressFamilies = [
          "AF_UNIX"
          "AF_INET"
          "AF_INET6"
        ];
        IPAddressDeny = "any";
        IPAddressAllow = "localhost";
        RestrictNamespaces = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        SystemCallArchitectures = "native";
        SystemCallFilter = [ "@system-service" ];
        SystemCallErrorNumber = "EPERM";
      };
    };
  };
}
