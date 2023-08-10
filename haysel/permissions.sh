#! /usr/bin/env nix-shell
#! nix-shell -p bash -i bash

confirm() {
    # call with a prompt string or use a default
    read -r -p "${1:-Are you sure? [y/N]} " response
    case "$response" in
        [yY][eE][sS]|[yY]) 
            true
            ;;
        *)
            false
            ;;
    esac
}

echo "modifying disk $1"
confirm || exit 1
echo "modifying permissions..."
sudo chown $USER $DISK || exit 1
sudo chmod 700 $DISK || exit 1

