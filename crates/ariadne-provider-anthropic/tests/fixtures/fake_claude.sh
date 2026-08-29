#!/bin/sh
if [ "$1" = "--version" ]; then
  printf '%s\n' '2.1.223 (Claude Code)'
  exit 0
fi

case "${ARIADNE_TEST_SCENARIO-}" in
  headless)
    printf '%s\n' "$@" > "$ARIADNE_TEST_ARGS"
    cat > "$ARIADNE_TEST_STDIN"
    printf '%s\n' \
      '{"type":"assistant","message":{"content":[{"type":"text","text":"Hello from subscription"}]}}' \
      '{"type":"result","subtype":"success","result":"Hello from subscription"}'
    ;;
  isolated)
    cat >/dev/null
    printf '%s' "${ANTHROPIC_API_KEY-unset}|${ANTHROPIC_AUTH_TOKEN-unset}|${ANTHROPIC_BASE_URL-unset}|${ANTHROPIC_PROFILE-unset}|${ANTHROPIC_FEDERATION_RULE_ID-unset}|${CLAUDE_CODE_USE_BEDROCK-unset}|${CLAUDE_CODE_USE_VERTEX-unset}|${CLAUDE_CODE_USE_FOUNDRY-unset}|${OPENAI_API_KEY-unset}|${AWS_SECRET_ACCESS_KEY-unset}|${CLAUDE_CODE_OAUTH_TOKEN-unset}" > "$ARIADNE_TEST_ENV"
    pwd > "$ARIADNE_TEST_CWD"
    printf '%s\n' \
      '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello"}}}' \
      '{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"}]}}' \
      '{"type":"result","subtype":"success","result":"Hello"}'
    ;;
  no-result)
    cat >/dev/null
    printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"incomplete"}]}}'
    ;;
  oversized)
    cat >/dev/null
    head -c 70000 /dev/zero | tr '\000' x
    ;;
  blocked-stdin)
    exec sleep 600
    ;;
  invalid-json-hang)
    printf '%s\n' "$$" > "$ARIADNE_TEST_PID"
    cat >/dev/null
    printf '%s\n' 'not-json'
    exec sleep 600
    ;;
  success-hang)
    printf '%s\n' "$$" > "$ARIADNE_TEST_PID"
    cat >/dev/null
    printf '%s\n' \
      '{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"}]}}' \
      '{"type":"result","subtype":"success","result":"Hello"}'
    exec sleep 600 >/dev/null
    ;;
  *)
    exit 99
    ;;
esac
