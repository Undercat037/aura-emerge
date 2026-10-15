# aura-emerge eselect shim — only the profile module
complete -c eselect -e 2>/dev/null
complete -c eselect -f
complete -c eselect -n '__fish_use_subcommand' -a profile -d 'make.profile symlink'
complete -c eselect -n '__fish_seen_subcommand_from profile' -a list -d 'list profiles'
complete -c eselect -n '__fish_seen_subcommand_from profile' -a set -d 'set make.profile'
complete -c eselect -n '__fish_seen_subcommand_from profile' -a show -d 'show current profile'
