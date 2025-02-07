#!/bin/sh
set -e

# Configuration
INSTALL_DIR="$HOME/.windchime"
BIN_DIR="$INSTALL_DIR/bin"
REPO_OWNER="nik"  # Change this to your GitHub username
REPO_NAME="windchime"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Print with color
print_info() {
    printf "${BLUE}info${NC} %s\n" "$1"
}

print_success() {
    printf "${GREEN}success${NC} %s\n" "$1"
}

print_error() {
    printf "${RED}error${NC} %s\n" "$1" >&2
}

# Detect operating system and architecture
detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"
    
    case "$OS" in
        Darwin) OS="macos" ;;
        Linux) OS="linux" ;;
        *) print_error "Unsupported operating system: $OS"; exit 1 ;;
    esac
    
    case "$ARCH" in
        x86_64) ARCH="x86_64" ;;
        aarch64|arm64) ARCH="aarch64" ;;
        *) print_error "Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    
    PLATFORM="${OS}-${ARCH}"
}

# Create installation directory
create_dirs() {
    mkdir -p "$BIN_DIR"
}

# Download the latest release
download_binary() {
    print_info "Downloading windchime..."
    
    # Get the latest release URL
    RELEASE_URL="https://github.com/$REPO_OWNER/$REPO_NAME/releases/latest/download/windchime-$PLATFORM.tar.gz"
    
    # Download and extract
    curl -L "$RELEASE_URL" | tar xz -C "$BIN_DIR"
    chmod +x "$BIN_DIR/windchime"
}

# Add to PATH in shell config
update_path() {
    # Detect shell
    SHELL_NAME="$(basename "$SHELL")"
    
    case "$SHELL_NAME" in
        bash)
            SHELL_CONFIG="$HOME/.bashrc"
            ;;
        zsh)
            SHELL_CONFIG="$HOME/.zshrc"
            ;;
        *)
            print_error "Unsupported shell: $SHELL_NAME"
            print_info "Please add $BIN_DIR to your PATH manually"
            return
            ;;
    esac
    
    # Add to PATH if not already present
    if ! echo "$PATH" | grep -q "$BIN_DIR"; then
        echo "export PATH=\"\$PATH:$BIN_DIR\"" >> "$SHELL_CONFIG"
        print_info "Added windchime to PATH in $SHELL_CONFIG"
    fi
}

# Main installation
main() {
    print_info "Installing windchime..."
    
    detect_platform
    create_dirs
    download_binary
    update_path
    
    print_success "windchime has been installed successfully!"
    print_info "Please restart your terminal or run: source $SHELL_CONFIG"
    print_info "Then you can start using windchime by running: windchime"
}

main 