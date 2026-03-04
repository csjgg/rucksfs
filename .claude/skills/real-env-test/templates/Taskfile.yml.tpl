# =============================================================================
# {{PROJECT_NAME}} Automated Deployment Testing
# =============================================================================
# Prerequisites:
#   - Install task: https://taskfile.dev/installation/
#   - {{BUILD_PREREQUISITES}}
#   - Copy env.example.yml to env.yml and fill in your values
#
# Quick start:
#   cp env.example.yml env.yml && vim env.yml && task test
#
# Agent workflow (async):
#   task build && task deploy && task check && task service:restart
#   task bench:start       # returns immediately
#   task bench:status      # poll until exit 0
#   task bench:stop && task collect
# =============================================================================

version: "3"

dotenv: []

vars:
  # ---------------------------------------------------------------------------
  # Load env.yml values (yq with grep/awk fallback)
  # ---------------------------------------------------------------------------
  REMOTE_HOST:
    sh: "yq -r '.remote.host' env.yml 2>/dev/null || grep 'host:' env.yml | head -1 | awk '{print $2}' | tr -d '\"'"
  REMOTE_USER:
    sh: "yq -r '.remote.user' env.yml 2>/dev/null || grep 'user:' env.yml | head -1 | awk '{print $2}' | tr -d '\"'"
  REMOTE_SSH_KEY:
    sh: "yq -r '.remote.ssh_key' env.yml 2>/dev/null || grep 'ssh_key:' env.yml | head -1 | awk '{print $2}' | tr -d '\"'"
  REMOTE_PORT:
    sh: "yq -r '.remote.port // 22' env.yml 2>/dev/null || echo '22'"

  # {{PROJECT_SPECIFIC_VARS}}
  # Add project-specific variables here (e.g., mount points, service config)

  # Benchmark variables
  BENCH_MODE:
    sh: "yq -r '.benchmark.mode // \"{{DEFAULT_BENCH_MODE}}\"' env.yml 2>/dev/null || echo '{{DEFAULT_BENCH_MODE}}'"
  BENCH_RATE:
    sh: "yq -r '.benchmark.rate // 1' env.yml 2>/dev/null || echo '1'"
  BENCH_DURATION:
    sh: "yq -r '.benchmark.duration // 60' env.yml 2>/dev/null || echo '60'"
  BENCH_DATASET:
    sh: "yq -r '.benchmark.dataset // \"{{DEFAULT_DATASET}}\"' env.yml 2>/dev/null || echo '{{DEFAULT_DATASET}}'"
  BENCH_NAMESPACE:
    sh: "yq -r '.benchmark.namespace // \"default\"' env.yml 2>/dev/null || echo 'default'"

  # Bastion (jump host) variables
  BASTION_ENABLED:
    sh: "yq -r '.bastion.enabled | if . == null then false else . end' env.yml 2>/dev/null || echo 'false'"
  BASTION_HOST:
    sh: "yq -r '.bastion.host // \"\"' env.yml 2>/dev/null || echo ''"
  BASTION_USER:
    sh: "yq -r '.bastion.user // \"root\"' env.yml 2>/dev/null || echo 'root'"
  BASTION_SSH_KEY:
    sh: "yq -r '.bastion.ssh_key // \"~/.ssh/id_rsa\"' env.yml 2>/dev/null || echo '~/.ssh/id_rsa'"
  BASTION_PORT:
    sh: "yq -r '.bastion.port // 22' env.yml 2>/dev/null || echo '22'"

  PROXY_JUMP_FLAG:
    sh: |
      if [ "{{.BASTION_ENABLED}}" = "true" ] && [ -n "{{.BASTION_HOST}}" ]; then
        echo "-J {{.BASTION_USER}}@{{.BASTION_HOST}}:{{.BASTION_PORT}}"
      else
        echo ""
      fi
  BASTION_KEY_FLAG:
    sh: |
      if [ "{{.BASTION_ENABLED}}" = "true" ] && [ -n "{{.BASTION_HOST}}" ]; then
        echo "-o IdentitiesOnly=yes"
      else
        echo ""
      fi

  # Derived variables
  SSH_OPTS: "-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"
  RESULTS_DIR: "{{.ROOT_DIR}}/results"
  SCRIPTS_DIR: "{{.ROOT_DIR}}/scripts"
  REMOTE_WORK_DIR: "/opt/benchmark"
  REMOTE_METRICS_DIR: "/tmp/benchmark-metrics"
  REMOTE_LOGS_DIR: "/tmp/benchmark-logs"
  TIMESTAMP:
    sh: "date +%Y%m%d_%H%M%S"

env:
  SSH_CMD: 'ssh {{.SSH_OPTS}} {{.PROXY_JUMP_FLAG}} {{.BASTION_KEY_FLAG}} -i {{.REMOTE_SSH_KEY}} -p {{.REMOTE_PORT}} {{.REMOTE_USER}}@{{.REMOTE_HOST}}'
  SCP_CMD: 'scp {{.SSH_OPTS}} {{.PROXY_JUMP_FLAG}} {{.BASTION_KEY_FLAG}} -i {{.REMOTE_SSH_KEY}} -P {{.REMOTE_PORT}}'

# =============================================================================
# Tasks
# =============================================================================
tasks:

  # --- Validation ---
  validate:
    desc: "Validate env.yml exists and has required fields"
    silent: true
    cmds:
      - |
        if [ ! -f env.yml ]; then
          echo "ERROR: env.yml not found. Copy env.example.yml to env.yml and configure."
          exit 1
        fi
      - |
        if [ -z "{{.REMOTE_HOST}}" ] || [ "{{.REMOTE_HOST}}" = "null" ]; then
          echo "ERROR: remote.host not set in env.yml"
          exit 1
        fi
      - |
        echo "Configuration:"
        echo "  Remote:     {{.REMOTE_USER}}@{{.REMOTE_HOST}}:{{.REMOTE_PORT}}"
        echo "  SSH Key:    {{.REMOTE_SSH_KEY}}"
        # {{VALIDATE_PROJECT_SPECIFIC}}

  # --- Build ---
  # {{BUILD_TASKS}}
  # Replace with project-specific build tasks. Example:
  #
  # build:component:
  #   desc: "Build <component>"
  #   dir: "{{.ROOT_DIR}}/../<component>"
  #   cmds:
  #     - echo "==> Building <component>..."
  #     - <build command>
  #
  # build:
  #   desc: "Build all"
  #   deps: [build:component]

  # --- Deploy ---
  deploy:upload:
    desc: "Upload binaries, scripts, and data to remote host"
    deps: [validate]
    cmds:
      - $SSH_CMD "mkdir -p {{.REMOTE_WORK_DIR}} /tmp"
      # {{UPLOAD_BINARIES}}
      # {{UPLOAD_SCRIPTS}}
      - |
        echo "==> Uploading scripts..."
        $SCP_CMD "{{.SCRIPTS_DIR}}/remote-setup.sh" {{.REMOTE_USER}}@{{.REMOTE_HOST}}:/tmp/remote-setup.sh
        $SCP_CMD "{{.SCRIPTS_DIR}}/collect-metrics.sh" {{.REMOTE_USER}}@{{.REMOTE_HOST}}:/tmp/collect-metrics.sh
        $SCP_CMD "{{.SCRIPTS_DIR}}/collect-logs.sh" {{.REMOTE_USER}}@{{.REMOTE_HOST}}:/tmp/collect-logs.sh
        $SSH_CMD "chmod +x /tmp/remote-setup.sh /tmp/collect-metrics.sh /tmp/collect-logs.sh"
      - echo "==> Upload complete"

  deploy:install:
    desc: "Install binaries on remote host"
    deps: [validate]
    cmds:
      - |
        echo "==> Installing on remote host..."
        $SSH_CMD "bash /tmp/remote-setup.sh install"

  deploy:configure:
    desc: "Configure services on remote host"
    deps: [validate]
    cmds:
      # {{CONFIGURE_COMMANDS}}
      - echo "==> Configuration complete"

  deploy:
    desc: "Full deploy: upload + install + configure"
    cmds:
      - task: deploy:upload
      - task: deploy:install
      - task: deploy:configure

  # --- Check ---
  check:
    desc: "Run environment checks on remote host"
    deps: [validate]
    cmds:
      - |
        echo "==> Running environment checks..."
        $SSH_CMD "bash /tmp/remote-setup.sh check"

  # --- Service Management ---
  service:start:
    desc: "Start service on remote host"
    deps: [validate]
    cmds:
      - $SSH_CMD "systemctl start {{SERVICE_NAME}} && sleep 2 && systemctl status {{SERVICE_NAME}} --no-pager"

  service:stop:
    desc: "Stop service on remote host"
    deps: [validate]
    cmds:
      - $SSH_CMD "systemctl stop {{SERVICE_NAME}} 2>/dev/null; echo 'Stopped'"

  service:restart:
    desc: "Restart services on remote host"
    deps: [validate]
    cmds:
      - |
        echo "==> Restarting services..."
        # {{RESTART_COMMANDS}}
        # Example: restart main service + dependencies

  service:status:
    desc: "Show service status"
    deps: [validate]
    cmds:
      - $SSH_CMD "systemctl status {{SERVICE_NAME}} --no-pager 2>&1 || true"

  # --- Benchmark (Async - Agent friendly) ---
  bench:start:
    desc: "[Agent] Start benchmark in background (returns immediately with PID)"
    deps: [validate]
    cmds:
      - |
        echo "==> Starting metrics collection..."
        $SSH_CMD "bash /tmp/collect-metrics.sh start {{.REMOTE_METRICS_DIR}} 1"
      - |
        echo "==> Launching benchmark in background..."
        $SSH_CMD "cd {{.REMOTE_WORK_DIR}} && \
          nohup bash -c '{{BENCH_COMMAND}}' > {{.REMOTE_WORK_DIR}}/bench.log 2>&1 & \
          echo \$! > {{.REMOTE_WORK_DIR}}/bench.pid && \
          sleep 1 && \
          PID=\$(cat {{.REMOTE_WORK_DIR}}/bench.pid) && \
          if ps -p \${PID} > /dev/null 2>&1; then \
            echo \"benchmark_started pid=\${PID}\"; \
          else \
            echo \"ERROR: benchmark failed to start\"; \
            tail -20 {{.REMOTE_WORK_DIR}}/bench.log 2>/dev/null; \
            exit 1; \
          fi"
      - echo "==> Use 'task bench:status' to check progress."

  bench:status:
    desc: "[Agent] Check benchmark status (exit 0 = finished, exit 1 = running)"
    deps: [validate]
    cmds:
      - |
        $SSH_CMD 'PIDFILE={{.REMOTE_WORK_DIR}}/bench.pid;
          if [ ! -f "${PIDFILE}" ]; then
            echo "status=no_benchmark";
            exit 0;
          fi;
          PID=$(cat "${PIDFILE}");
          if ps -p ${PID} > /dev/null 2>&1; then
            ELAPSED=$(ps -p ${PID} -o etime= 2>/dev/null | tr -d " ");
            echo "status=running";
            echo "pid=${PID}";
            echo "elapsed=${ELAPSED}";
            exit 1;
          else
            echo "status=finished";
            echo "pid=${PID}";
            echo "--- benchmark output (last 15 lines) ---";
            tail -15 {{.REMOTE_WORK_DIR}}/bench.log 2>/dev/null || true;
            echo "--- result files ---";
            ls -lh {{.REMOTE_WORK_DIR}}/*.json 2>/dev/null || echo "No result files";
            rm -f "${PIDFILE}";
            exit 0;
          fi'

  bench:stop:
    desc: "[Agent] Stop metrics and collect logs (call after bench finishes)"
    deps: [validate]
    cmds:
      - $SSH_CMD "bash /tmp/collect-metrics.sh stop" || true
      - |
        echo "==> Collecting logs..."
        $SSH_CMD "bash /tmp/collect-logs.sh {{.REMOTE_LOGS_DIR}} {{.BENCH_DURATION}}" || true
      - echo "==> Post-benchmark collection complete"

  bench:run:
    desc: "[Sync] Run benchmark blocking until complete"
    deps: [validate]
    cmds:
      - $SSH_CMD "bash /tmp/collect-metrics.sh start {{.REMOTE_METRICS_DIR}} 1"
      - |
        echo "==> Running benchmark (blocking)..."
        $SSH_CMD "cd {{.REMOTE_WORK_DIR}} && {{BENCH_COMMAND}}" || true
      - $SSH_CMD "bash /tmp/collect-metrics.sh stop" || true
      - $SSH_CMD "bash /tmp/collect-logs.sh {{.REMOTE_LOGS_DIR}} {{.BENCH_DURATION}}" || true

  bench:
    desc: "Alias for bench:run"
    cmds: [{ task: bench:run }]

  # --- Collect ---
  collect:
    desc: "Pull results, metrics, and logs from remote host"
    deps: [validate]
    cmds:
      - |
        LOCAL_DIR="{{.RESULTS_DIR}}/run_{{.TIMESTAMP}}"
        mkdir -p "${LOCAL_DIR}/benchmark" "${LOCAL_DIR}/metrics" "${LOCAL_DIR}/logs"
        echo "==> Collecting results to ${LOCAL_DIR}..."
        $SCP_CMD "{{.REMOTE_USER}}@{{.REMOTE_HOST}}:{{.REMOTE_WORK_DIR}}/*.json" "${LOCAL_DIR}/benchmark/" 2>/dev/null || echo "No benchmark reports"
        $SCP_CMD -r "{{.REMOTE_USER}}@{{.REMOTE_HOST}}:{{.REMOTE_METRICS_DIR}}/*" "${LOCAL_DIR}/metrics/" 2>/dev/null || echo "No metrics"
        $SCP_CMD -r "{{.REMOTE_USER}}@{{.REMOTE_HOST}}:{{.REMOTE_LOGS_DIR}}/*" "${LOCAL_DIR}/logs/" 2>/dev/null || echo "No logs"
        echo "==> Results: ${LOCAL_DIR}"
        find "${LOCAL_DIR}" -type f -exec ls -lh {} \; 2>/dev/null || true

  # --- Cleanup ---
  clean:containers:
    desc: "Remove benchmark containers"
    deps: [validate]
    cmds:
      - |
        echo "==> Cleaning containers..."
        # {{CLEAN_CONTAINERS_CMD}}

  clean:images:
    desc: "Remove images"
    deps: [validate]
    cmds:
      - |
        echo "==> Removing images..."
        # {{CLEAN_IMAGES_CMD}}

  clean:mounts:
    desc: "Unmount project-specific mounts and detach loop devices"
    deps: [validate]
    cmds:
      - |
        echo "==> Cleaning mounts..."
        # {{CLEAN_MOUNTS_CMD}}

  clean:snapshotter-data:
    desc: "Remove metadata and snapshot data"
    deps: [validate]
    cmds:
      - |
        echo "==> Cleaning data..."
        # {{CLEAN_DATA_CMD}}

  clean:remote-data:
    desc: "Remove remote temp files"
    deps: [validate]
    cmds:
      - |
        echo "==> Cleaning remote data..."
        $SSH_CMD "rm -rf {{.REMOTE_WORK_DIR}}/*.json {{.REMOTE_WORK_DIR}}/bench.log \
          {{.REMOTE_WORK_DIR}}/bench.pid {{.REMOTE_METRICS_DIR}} {{.REMOTE_LOGS_DIR}} \
          /tmp/remote-setup.sh /tmp/collect-metrics.sh /tmp/collect-logs.sh"

  clean:rotate-dirs:
    desc: "Rotate data directories for pristine state"
    deps: [validate]
    cmds:
      - |
        echo "==> Rotating data directories..."
        # {{ROTATE_DIRS_CMD}}

  clean:reboot:
    desc: "Reboot remote host and wait for recovery"
    deps: [validate]
    cmds:
      - |
        echo "==> Rebooting {{.REMOTE_HOST}}..."
        $SSH_CMD "reboot" 2>/dev/null || true
        sleep 15
        RETRIES=0; MAX=20
        while [ ${RETRIES} -lt ${MAX} ]; do
          if $SSH_CMD "echo 'host_up'" 2>/dev/null | grep -q 'host_up'; then
            echo "==> Host back online!"
            sleep 5
            exit 0
          fi
          RETRIES=$((RETRIES + 1))
          echo "    Waiting... (${RETRIES}/${MAX})"
          sleep 10
        done
        echo "ERROR: Host did not recover"
        exit 1

  clean:
    desc: "Standard cleanup"
    cmds:
      - task: service:stop
      - task: clean:mounts
      - task: clean:containers
      - task: clean:images
      - task: clean:snapshotter-data
      - task: clean:remote-data

  clean:full-rotate:
    desc: "Deep cleanup: standard + rotate dirs + reboot"
    cmds:
      - task: clean
      - task: clean:rotate-dirs
      - task: clean:reboot

  # --- Composite Workflows ---
  test:
    desc: "[Sync] Full test flow (blocks until done)"
    cmds:
      - task: validate
      - task: build
      - task: deploy
      - task: check
      - task: service:restart
      - task: bench:run
      - task: collect

  test:agent:
    desc: "[Agent] Full test flow (async, returns after bench:start)"
    cmds:
      - task: validate
      - task: build
      - task: deploy
      - task: check
      - task: service:restart
      - task: bench:start
      - |
        echo "========================================="
        echo " Benchmark launched. Next:"
        echo "   1. task bench:status  (poll until exit 0)"
        echo "   2. task bench:stop"
        echo "   3. task collect"
        echo "========================================="

  quick-bench:
    desc: "[Sync] Re-benchmark (skip build/deploy)"
    cmds:
      - task: validate
      - task: clean:containers
      - task: clean:images
      - task: clean:mounts
      - task: service:restart
      - task: bench:run
      - task: collect

  quick-bench:agent:
    desc: "[Agent] Re-benchmark (skip build/deploy, async)"
    cmds:
      - task: validate
      - task: clean:containers
      - task: clean:images
      - task: clean:mounts
      - task: service:restart
      - task: bench:start

  clean-bench:agent:
    desc: "[Agent] Deep-clean + benchmark (pristine state, async)"
    cmds:
      - task: validate
      - task: clean:full-rotate
      - task: deploy:configure
      - task: service:restart
      - task: bench:start

  # --- Utility ---
  ssh:
    desc: "Open interactive SSH session"
    deps: [validate]
    cmds:
      - ssh {{.SSH_OPTS}} {{.PROXY_JUMP_FLAG}} {{.BASTION_KEY_FLAG}} -i {{.REMOTE_SSH_KEY}} -p {{.REMOTE_PORT}} {{.REMOTE_USER}}@{{.REMOTE_HOST}}
    interactive: true
