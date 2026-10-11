#!/bin/bash
# Build an app (fast profile), push it to Midget over the MCP HTTP endpoint, install and open it.
# Usage: tools/mcp-push-app.sh APP [--skip-build] [--no-open]   (APP = directory under springboard/apps)
exec python3 "$(dirname "$(readlink -f "$0")")/mcp-deploy-app.py" "$@"
