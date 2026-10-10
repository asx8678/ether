#!/usr/bin/env python3
# New in ether: prompt-cache prefix check over a request log (the idea of Codex's core/tests/suite/prompt_caching.rs).
"""check-prefix.py REQUEST_LOG

Reads the JSON lines written by `ether --request-log FILE` and checks the prompt-cache rules:

- the system prompt / instructions and the tool list are byte-identical in every request;
- each request's messages (chat) or input (responses) start with exactly the previous
  request's, unless the history was compacted in between (reported, not a failure).

Exits 1 on a violation.
"""
import json
import sys


def canon(value):
    return json.dumps(value, sort_keys=False, separators=(",", ":"), ensure_ascii=False)


def split(body):
    if "messages" in body:
        messages = body["messages"]
        head = [m for m in messages[:1] if m.get("role") == "system"]
        return canon(head), canon(body.get("tools", [])), messages[len(head):]
    return canon(body.get("instructions", "")), canon(body.get("tools", [])), body.get("input", [])


def is_compaction_request(history):
    last = history[-1] if history else {}
    text = canon(last.get("content", ""))
    return "CONTEXT CHECKPOINT" in text


def main(path):
    bodies = [json.loads(line) for line in open(path) if line.strip()]
    if len(bodies) < 2:
        print(f"{path}: {len(bodies)} request(s); nothing to compare")
        return 0
    failures = 0
    previous = None
    for index, body in enumerate(bodies, start=1):
        system, tools, history = split(body)
        if previous is not None:
            p_system, p_tools, p_history, p_index = previous
            if is_compaction_request(history) or is_compaction_request(p_history):
                print(f"request {index}: compaction request, prefix not compared")
            else:
                if system != p_system:
                    print(f"request {index}: system prompt differs from request {p_index}")
                    failures += 1
                if tools != p_tools:
                    print(f"request {index}: tools differ from request {p_index}")
                    failures += 1
                prefix = history[: len(p_history)]
                if [canon(m) for m in prefix] != [canon(m) for m in p_history]:
                    first = next(
                        (i for i, (a, b) in enumerate(zip(prefix, p_history)) if canon(a) != canon(b)),
                        min(len(prefix), len(p_history)),
                    )
                    print(
                        f"request {index}: history does not start with request {p_index}'s "
                        f"(first difference at item {first})"
                    )
                    failures += 1
                else:
                    print(
                        f"request {index}: ok, starts with request {p_index}'s {len(p_history)} "
                        f"items, adds {len(history) - len(p_history)}"
                    )
        previous = (system, tools, history, index)
    print("prefix check:", "FAILED" if failures else "passed")
    return 1 if failures else 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    sys.exit(main(sys.argv[1]))
