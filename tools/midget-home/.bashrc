# ~/.bashrc for root on Midget (BifrOSt). Deployed by tools/midget-home/deploy.sh.
# Midget has no Omarchy, so this file does not source Omarchy; keep it self-contained.

# Non-interactive shells stop here, after PATH is set.
export PATH="/usr/local/sbin:/usr/local/bin:/usr/bin:/usr/sbin:/sbin:/bin"
[[ -d $HOME/.cargo/bin ]] && export PATH="$HOME/.cargo/bin:$PATH"
[[ -d $HOME/.local/bin ]] && export PATH="$HOME/.local/bin:$PATH"

[[ $- != *i* ]] && return

export EDITOR=${EDITOR:-vi}
export HISTSIZE=5000 HISTFILESIZE=10000 HISTCONTROL=ignoreboth:erasedups
shopt -s histappend checkwinsize 2>/dev/null

alias ls='ls --color=auto'
alias ll='ls -lah --color=auto'
alias grep='grep --color=auto'
alias ..='cd ..'

PS1='\[\e[1;31m\]\u@midget\[\e[0m\]:\[\e[1;34m\]\w\[\e[0m\]\$ '

# Shared system bash completion, when installed.
[[ -r /usr/share/bash-completion/bash_completion ]] && . /usr/share/bash-completion/bash_completion
