#!/usr/bin/env bash
# =============================================================================
# testing/scripts/remote-setup.sh
# Remote host setup for RucksFS testing
#
# Subcommands:
#   install <work_dir>                      Install rucksfs binary to /usr/local/bin
#   configure <mountpoint> <data_dir>       Create dirs, configure FUSE
#   check <mountpoint> <data_dir>           Verify environment is ready
# =============================================================================

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

SUBCMD="${1:-help}"
shift || true

case "$SUBCMD" in
  install)
    WORK_DIR="${1:-/opt/rucksfs-test}"
    echo "==> Installing rucksfs..."

    if [ ! -f "${WORK_DIR}/rucksfs" ]; then
      echo -e "${RED}ERROR: ${WORK_DIR}/rucksfs not found. Upload first.${NC}"
      exit 1
    fi

    cp "${WORK_DIR}/rucksfs" /usr/local/bin/rucksfs
    chmod +x /usr/local/bin/rucksfs
    echo -e "${GREEN}Installed: $(which rucksfs)${NC}"
    rucksfs --version 2>/dev/null || echo "(version not available)"
    ;;

  configure)
    MOUNTPOINT="${1:-/mnt/rucksfs}"
    DATA_DIR="${2:-/var/lib/rucksfs}"

    echo "==> Configuring environment..."

    # Create directories
    mkdir -p "$MOUNTPOINT" "$DATA_DIR"
    echo "  Created: $MOUNTPOINT"
    echo "  Created: $DATA_DIR"

    # Check FUSE support
    if [ ! -e /dev/fuse ]; then
      echo -e "${YELLOW}WARNING: /dev/fuse not found. Loading fuse module...${NC}"
      modprobe fuse 2>/dev/null || true
      if [ ! -e /dev/fuse ]; then
        echo -e "${RED}ERROR: FUSE not available. Install fuse/fuse3 package.${NC}"
        exit 1
      fi
    fi
    echo -e "  ${GREEN}/dev/fuse: OK${NC}"

    # Configure user_allow_other
    FUSE_CONF="/etc/fuse.conf"
    if [ -f "$FUSE_CONF" ]; then
      if ! grep -q "^user_allow_other" "$FUSE_CONF"; then
        echo "user_allow_other" >> "$FUSE_CONF"
        echo "  Added user_allow_other to $FUSE_CONF"
      else
        echo "  user_allow_other already in $FUSE_CONF"
      fi
    else
      echo "user_allow_other" > "$FUSE_CONF"
      echo "  Created $FUSE_CONF with user_allow_other"
    fi

    # Install essential tools if missing
    for tool in bc time stat md5sum; do
      if ! command -v "$tool" &>/dev/null; then
        echo -e "  ${YELLOW}Installing $tool...${NC}"
        apt-get install -y "$tool" 2>/dev/null || yum install -y "$tool" 2>/dev/null || true
      fi
    done

    echo -e "${GREEN}==> Configuration complete${NC}"
    ;;

  check)
    MOUNTPOINT="${1:-/mnt/rucksfs}"
    DATA_DIR="${2:-/var/lib/rucksfs}"
    ERRORS=0

    echo "==> Environment checks..."

    # Check rucksfs binary
    if command -v rucksfs &>/dev/null; then
      echo -e "  ${GREEN}[OK]${NC} rucksfs binary found: $(which rucksfs)"
    else
      echo -e "  ${RED}[FAIL]${NC} rucksfs binary not in PATH"
      ERRORS=$((ERRORS + 1))
    fi

    # Check FUSE
    if [ -e /dev/fuse ]; then
      echo -e "  ${GREEN}[OK]${NC} /dev/fuse available"
    else
      echo -e "  ${RED}[FAIL]${NC} /dev/fuse not found"
      ERRORS=$((ERRORS + 1))
    fi

    # Check fuse.conf
    if grep -q "^user_allow_other" /etc/fuse.conf 2>/dev/null; then
      echo -e "  ${GREEN}[OK]${NC} user_allow_other enabled"
    else
      echo -e "  ${YELLOW}[WARN]${NC} user_allow_other not in /etc/fuse.conf"
    fi

    # Check fusermount
    if command -v fusermount &>/dev/null || command -v fusermount3 &>/dev/null; then
      echo -e "  ${GREEN}[OK]${NC} fusermount available"
    else
      echo -e "  ${RED}[FAIL]${NC} fusermount not found"
      ERRORS=$((ERRORS + 1))
    fi

    # Check directories
    if [ -d "$MOUNTPOINT" ]; then
      echo -e "  ${GREEN}[OK]${NC} Mountpoint exists: $MOUNTPOINT"
    else
      echo -e "  ${YELLOW}[WARN]${NC} Mountpoint missing: $MOUNTPOINT (will be created)"
    fi

    if [ -d "$DATA_DIR" ]; then
      echo -e "  ${GREEN}[OK]${NC} Data dir exists: $DATA_DIR"
    else
      echo -e "  ${YELLOW}[WARN]${NC} Data dir missing: $DATA_DIR (will be created)"
    fi

    # Check required tools for benchmark
    for tool in bash bc stat md5sum dd; do
      if command -v "$tool" &>/dev/null; then
        echo -e "  ${GREEN}[OK]${NC} $tool"
      else
        echo -e "  ${YELLOW}[WARN]${NC} $tool not found (some benchmarks may skip)"
      fi
    done

    # Check optional tools
    echo ""
    echo "  Optional tools:"
    for tool in python3 fio pjdfstest; do
      if command -v "$tool" &>/dev/null; then
        echo -e "  ${GREEN}[OK]${NC} $tool"
      else
        echo -e "  ${YELLOW}[--]${NC} $tool not installed (optional)"
      fi
    done

    # Disk space
    echo ""
    echo "  Disk space:"
    df -h "$(dirname $DATA_DIR)" 2>/dev/null | tail -1

    if [ $ERRORS -gt 0 ]; then
      echo ""
      echo -e "${RED}$ERRORS critical check(s) failed${NC}"
      exit 1
    fi

    echo ""
    echo -e "${GREEN}All critical checks passed${NC}"
    ;;

  help|*)
    echo "Usage: $0 <subcommand> [args]"
    echo ""
    echo "Subcommands:"
    echo "  install <work_dir>                    Install rucksfs binary"
    echo "  configure <mountpoint> <data_dir>     Set up FUSE and directories"
    echo "  check <mountpoint> <data_dir>         Verify environment"
    ;;
esac
