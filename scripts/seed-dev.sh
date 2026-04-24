#!/usr/bin/env bash
set -euo pipefail
mkdir -p fixtures/yaml
cat > fixtures/yaml/app.yaml <<YAML
service:
  name: demo
  replicas: 2
YAML
