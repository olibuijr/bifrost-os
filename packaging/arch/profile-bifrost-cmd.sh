# BifrOSt command-line tools whose names an Arch package already owns (packaging/arch/package-files.sh) live in
# /usr/lib/bifrost/cmd/bin. /usr/bin stays ahead of it in PATH, so the Arch file keeps its name.
case ":$PATH:" in
  *":/usr/lib/bifrost/cmd/bin:"*) ;;
  *) PATH="$PATH:/usr/lib/bifrost/cmd/bin" ;;
esac
export PATH
