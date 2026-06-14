#!/usr/bin/env python3
"""
LLM Provider Fixture 采集脚本

输入：provider + model + protocol + api_key
输出：tests/fixtures/llm/v2/{provider}/{model}/{protocol}/{scenario}.(json|txt)

规则：
- 每个 (model, protocol) 组合自动跑全部适用场景（不提供单独跑一个场景的功能）
- 场景不支持该模型时，仍输出错误信息文件（不跳过任何一个场景）
- 流式场景输出 .txt + -meta.json；非流式输出 .json
"""

import argparse
import json
import os
import sys
import time
from abc import ABC, abstractmethod
from pathlib import Path
from typing import Any

import urllib.request
import urllib.error

from providers import PROVIDER_CONFIG


# ============================================================
# HTTP 底层
# ============================================================

class HTTPResponseError(Exception):
    def __init__(self, code: int, reason: str, body: dict | str):
        super().__init__(f"HTTP {code} {reason}: {body}")
        self.code = code
        self.reason = reason
        self.body = body


def _http_post(url: str, headers: dict, payload: dict, timeout: int = 60) -> dict:
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(url, data=data, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8") if e.fp else ""
        try:
            err_body = json.loads(body)
        except Exception:
            err_body = {"raw": body}
        raise HTTPResponseError(e.code, e.reason, err_body)


def _http_post_streaming(url: str, headers: dict, payload: dict, timeout: int = 120) -> str:
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(url, data=data, headers=headers, method="POST")
    lines = []
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        for line in resp:
            if line:
                lines.append(line.decode("utf-8").rstrip())
    return "\n".join(lines) + "\ndata: [DONE]\n"


def _http_get(url: str, headers: dict, timeout: int = 30) -> dict:
    req = urllib.request.Request(url, headers=headers, method="GET")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8") if e.fp else ""
        try:
            err_body = json.loads(body)
        except Exception:
            err_body = {"raw": body}
        raise HTTPResponseError(e.code, e.reason, err_body)


# ============================================================
# 协议抽象层
# ============================================================

class LLMClient(ABC):
    @abstractmethod
    def chat(self, messages: list[dict], model: str, **kwargs) -> dict:
        ...

    @abstractmethod
    def chat_streaming(self, messages: list[dict], model: str, **kwargs) -> str:
        ...


class OpenAIClient(LLMClient):
    def __init__(self, base_url: str, api_key: str, error_via_base_resp: bool = False):
        self.base_url = base_url.rstrip("/")
        self._error_via_base_resp = error_via_base_resp
        self.headers = {
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        }

    def chat(self, messages: list[dict], model: str, **kwargs) -> dict:
        payload: dict[str, Any] = {"model": model, "messages": messages, **kwargs}
        resp = _http_post(self.base_url, self.headers, payload)
        if self._error_via_base_resp:
            if "base_resp" in resp and resp["base_resp"].get("status_code", 0) != 0:
                raise HTTPResponseError(200, "业务错误", resp)
        return resp

    def chat_streaming(self, messages: list[dict], model: str, **kwargs) -> str:
        payload: dict[str, Any] = {
            "model": model,
            "messages": messages,
            "stream": True,
            **kwargs,
        }
        return _http_post_streaming(self.base_url, self.headers, payload)


class AnthropicClient(LLMClient):
    def __init__(self, base_url: str, api_key: str):
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.headers = {
            "x-api-key": api_key,
            "anthropic-version": "2023-06-01",
            "Content-Type": "application/json",
        }

    def chat(self, messages: list[dict], model: str, **kwargs) -> dict:
        payload: dict[str, Any] = {"model": model, "messages": messages, **kwargs}
        return _http_post(self.base_url, self.headers, payload)

    def chat_streaming(self, messages: list[dict], model: str, **kwargs) -> str:
        payload: dict[str, Any] = {
            "model": model,
            "messages": messages,
            "stream": True,
            **kwargs,
        }
        return _http_post_streaming(self.base_url, self.headers, payload)


def make_client(provider: str, api_key: str, protocol: str) -> LLMClient:
    cfg = PROVIDER_CONFIG[provider]
    if protocol == "openai":
        return OpenAIClient(
            cfg["openai_url"],
            api_key,
            error_via_base_resp=cfg.get("_error_via_base_resp", False),
        )
    else:
        url = cfg.get("anthropic_url")
        if not url:
            raise ValueError(f"Provider {provider} 不支持 Anthropic 协议")
        return AnthropicClient(url, api_key)


# ============================================================
# 工具定义
# ============================================================

OPENAI_TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "Get the current weather in a given location.",
            "parameters": {
                "type": "object",
                "properties": {
                    "location": {"type": "string", "description": "City name, e.g. Tokyo"},
                },
                "required": ["location"],
            },
        },
    }
]

ANTHROPIC_TOOLS = [
    {
        "name": "get_weather",
        "description": "Get the current weather in a given location.",
        "input_schema": {
            "type": "object",
            "properties": {
                "location": {"type": "string", "description": "City name, e.g. Tokyo"},
            },
            "required": ["location"],
        },
    }
]

# 长 system prompt（300+ tokens），用于 cache-long 场景
LONG_SYSTEM_PROMPT = (
    "You are an expert software engineer and coding assistant with deep knowledge of "
    "multiple programming languages, software architecture, and best practices. Your role "
    "is to help users write, review, and debug code across various languages including but "
    "not limited to Rust, Python, TypeScript, Go, and Java. You follow industry-standard "
    "coding conventions, emphasize readability and maintainability, and always consider "
    "edge cases and error handling. When suggesting code changes, you provide clear "
    "explanations of why the change is necessary and how it improves the existing codebase. "
    "You are familiar with modern development tools, CI/CD pipelines, testing frameworks, "
    "and deployment strategies. You understand the importance of security, performance, and "
    "scalability in software design. When faced with ambiguous requirements, you ask "
    "clarifying questions before proceeding with implementation. You prefer simple, elegant "
    "solutions over clever but hard-to-understand code. You document your reasoning and "
    "decisions clearly. You are patient with learners and thorough in your explanations, "
    "breaking down complex concepts into digestible parts. You stay current with the latest "
    "developments in the software engineering field and can discuss trade-offs between "
    "different approaches."
)


# ============================================================
# 非 Chat 场景定义
# ============================================================

NON_CHAT_SCENARIOS: dict[str, dict[str, Any]] = {
    "model-list": {
        "description": "获取 provider 模型列表",
        "provider_url_key": "models_url",
        "anthropic_url_key": "anthropic_models_url",
    },
    "usage-quota": {
        "description": "查询用量/配额",
        "provider_url_key": "usage_url",
    },
}


# ============================================================
# 场景定义（每个场景都有 protocol + provider 过滤条件）
# ============================================================

SCENARIOS: dict[str, dict[str, Any]] = {
    # ---------- OpenAI 通用（所有 provider）----------
    "simple": {
        "protocol": "openai",
        "messages": [{"role": "user", "content": "Say hello in 3 words."}],
        "expect": "text",
    },
    "streaming": {
        "protocol": "openai",
        "messages": [{"role": "user", "content": "Count from 1 to 3."}],
        "expect": "streaming",
    },
    "multi-turn": {
        "protocol": "openai",
        "messages": [
            {"role": "user", "content": "My name is Alice."},
            {"role": "assistant", "content": "Hi Alice! Nice to meet you."},
            {"role": "user", "content": "What is my name?"},
        ],
        "expect": "context",
    },
    "context-pressure": {
        "protocol": "openai",
        "messages": [
            {"role": "system", "content": LONG_SYSTEM_PROMPT},
            {"role": "user", "content": "What is 2+2? Answer in one sentence."},
        ],
        "expect": "context_pressure",
        "_context_pressure": True,
        "_context_turns": 5,
        "_context_followups": [
            "Now explain the Riemann hypothesis and its implications for prime number distribution.",
            "Compare and contrast TCP and UDP protocols, including congestion control and use cases.",
            "Describe the CAP theorem in distributed systems and give examples of each trade-off.",
            "Explain how transformer attention mechanisms work, from matrix multiplication to multi-head attention.",
        ],
        "_max_tokens_override": 100,
    },
    "cache": {
        "protocol": "openai",
        "messages": [
            {"role": "system", "content": LONG_SYSTEM_PROMPT},
            {"role": "user", "content": "解释 HTTP/2 多路复用的工作原理，以及它相比 HTTP/1.1 在性能上有哪些改进。举一个具体例子。"},
        ],
        "expect": "cache_incremental",
        "_context_pressure": True,
        "_context_turns": 5,
        "_context_followups": [
            "详细解释 RAFT 共识算法，包括领导者选举和日志复制的过程。",
            "描述 TLS 1.3 握手流程，并与 TLS 1.2 对比，说明安全性和性能上的改进。",
        ],
        "_revert_after_turn": 1,
        "_revert_at_turn": 4,
        "_revert_user": "解释 Linux 内存的 slab 分配器和 buddy 系统，以及缺页中断的处理流程。",
        "_revert_followups": ["对比 B+ 树和 LSM 树在数据库索引中的优缺点，各自适合什么场景？"],
        "_max_tokens_override": 300,
    },
    "tool-use": {
        "protocol": "openai",
        "messages": [{"role": "user", "content": "How's the weather in San Francisco?"}],
        "expect": "tool_calls",
        "tools": OPENAI_TOOLS,
    },
    "tool-use-streaming": {
        "protocol": "openai",
        "messages": [{"role": "user", "content": "How's the weather in San Francisco?"}],
        "expect": "tool_calls_streaming",
        "tools": OPENAI_TOOLS,
    },
    "tool-result": {
        "protocol": "openai",
        "messages": [{"role": "user", "content": "How's the weather in San Francisco?"}],
        "expect": "tool_result",
        "tools": OPENAI_TOOLS,
        "extra_body": {"reasoning_split": True},
        "_mock_tool_response": "24\u2103, sunny",
    },
    "error-auth": {
        "protocol": "openai",
        "messages": [{"role": "user", "content": "hi"}],
        "expect": "error",
        "_invalid_key": True,
    },
    "error-model": {
        "protocol": "openai",
        "messages": [{"role": "user", "content": "hi"}],
        "expect": "error",
        "_invalid_model": "this-model-does-not-exist-999",
    },
    "error-empty": {
        "protocol": "openai",
        "messages": [],
        "expect": "error",
    },
    "error-tool-format": {
        "protocol": "openai",
        "messages": [{"role": "user", "content": "hi"}],
        "expect": "error",
        "_tools_invalid": [
            {
                "type": "function",
                "function": {
                    "description": "test",
                    "parameters": {"type": "object", "properties": {}},
                }
            }
        ],
    },
    # ---------- OpenAI + thinking（各 provider 专用协议）----------
    "minimax-reasoning-split": {
        "protocol": "openai",
        "provider": "minimax",
        "messages": [{"role": "user", "content": "What is 17*23? Show your work step by step."}],
        "expect": "reasoning",
        "extra_body": {"reasoning_split": True},
    },
    "glm-thinking": {
        "protocol": "openai",
        "provider": "glm",
        "messages": [{"role": "user", "content": "What is 17*23? Show your work step by step."}],
        "expect": "reasoning",
        "extra_body": {"thinking": {"type": "enabled"}},
    },
    "glm-thinking-disabled": {
        "protocol": "openai",
        "provider": "glm",
        "messages": [{"role": "user", "content": "What is 17*23?"}],
        "expect": "no-reasoning",
        "extra_body": {"thinking": {"type": "disabled"}},
    },
    "deepseek-thinking-high": {
        "protocol": "openai",
        "provider": "deepseek",
        "messages": [{"role": "user", "content": "What is 17*23? Show your work step by step."}],
        "expect": "reasoning",
        "_top_level": {"reasoning_effort": "high"},
    },
    "deepseek-thinking-disabled": {
        "protocol": "openai",
        "provider": "deepseek",
        "messages": [{"role": "user", "content": "What is 17*23?"}],
        "expect": "no-reasoning",
        "_top_level": {"reasoning_effort": "low"},
    },
    # ---------- Anthropic 通用（所有 provider）----------
    "anthropic-simple": {
        "protocol": "anthropic",
        "messages": [{"role": "user", "content": "Say hello in 3 words."}],
        "expect": "text",
        "max_tokens": 1024,
    },
    "anthropic-thinking": {
        "protocol": "anthropic",
        "messages": [{"role": "user", "content": "What is 17*23? Show your work step by step."}],
        "expect": "reasoning",
        "max_tokens": 2048,
    },
    "anthropic-streaming": {
        "protocol": "anthropic",
        "messages": [{"role": "user", "content": "Count from 1 to 3."}],
        "expect": "streaming",
        "max_tokens": 1024,
    },
    "anthropic-tool-use": {
        "protocol": "anthropic",
        "messages": [{"role": "user", "content": "How's the weather in San Francisco?"}],
        "expect": "tool_use",
        "tools": ANTHROPIC_TOOLS,
        "max_tokens": 2048,
    },
    "anthropic-tool-result": {
        "protocol": "anthropic",
        "messages": [{"role": "user", "content": "How's the weather in San Francisco?"}],
        "expect": "tool_result",
        "tools": ANTHROPIC_TOOLS,
        "max_tokens": 2048,
        "_mock_tool_response": "24\u2103, sunny",
    },
    "anthropic-tool-use-streaming": {
        "protocol": "anthropic",
        "messages": [{"role": "user", "content": "How's the weather in San Francisco?"}],
        "expect": "tool_use_streaming",
        "tools": ANTHROPIC_TOOLS,
        "max_tokens": 2048,
    },
    "anthropic-cache": {
        "protocol": "anthropic",
        "messages": [
            {"role": "user", "content": "解释 HTTP/2 多路复用的工作原理，以及它相比 HTTP/1.1 在性能上有哪些改进。举一个具体例子。"},
        ],
        "expect": "cache_incremental",
        "_context_pressure": True,
        "_context_turns": 5,
        "_context_followups": [
            "详细解释 RAFT 共识算法，包括领导者选举和日志复制的过程。",
            "描述 TLS 1.3 握手流程，并与 TLS 1.2 对比，说明安全性和性能上的改进。",
        ],
        "_revert_after_turn": 1,
        "_revert_at_turn": 4,
        "_revert_user": "解释 Linux 内存的 slab 分配器和 buddy 系统，以及缺页中断的处理流程。",
        "_revert_followups": ["对比 B+ 树和 LSM 树在数据库索引中的优缺点，各自适合什么场景？"],
        "system": [
            {"type": "text", "text": LONG_SYSTEM_PROMPT},
        ],
        "max_tokens": 300,
    },
    "anthropic-context-pressure": {
        "protocol": "anthropic",
        "messages": [
            {"role": "user", "content": "What is 2+2? Answer in one sentence."},
        ],
        "expect": "context_pressure",
        "_context_pressure": True,
        "_context_turns": 5,
        "_context_followups": [
            "Now explain the Riemann hypothesis and its implications for prime number distribution.",
            "Compare and contrast TCP and UDP protocols, including congestion control and use cases.",
            "Describe the CAP theorem in distributed systems and give examples of each trade-off.",
            "Explain how transformer attention mechanisms work, from matrix multiplication to multi-head attention.",
        ],
        "system": [
            {"type": "text", "text": LONG_SYSTEM_PROMPT},
        ],
        "max_tokens": 100,
    },
    "anthropic-error-auth": {
        "protocol": "anthropic",
        "messages": [{"role": "user", "content": "hi"}],
        "expect": "error",
        "max_tokens": 1024,
        "_invalid_key": True,
    },
    "anthropic-error-model": {
        "protocol": "anthropic",
        "messages": [{"role": "user", "content": "hi"}],
        "expect": "error",
        "max_tokens": 1024,
        "_invalid_model": "this-model-does-not-exist-999",
    },
    "anthropic-error-empty": {
        "protocol": "anthropic",
        "messages": [],
        "expect": "error",
        "max_tokens": 1024,
    },
}



# ============================================================
# 辅助函数
# ============================================================

def _build_request_kwargs(spec: dict[str, Any], messages: list[dict]) -> dict[str, Any]:
    kwargs: dict[str, Any] = {}
    protocol = spec["protocol"]

    if protocol == "anthropic":
        for field in ("max_tokens", "system", "temperature", "top_p"):
            if field in spec:
                kwargs[field] = spec[field]
        if spec.get("tools"):
            kwargs["tools"] = spec["tools"]
    else:
        for field in ("temperature", "top_p", "max_tokens"):
            if field in spec:
                kwargs[field] = spec[field]
        if spec.get("_max_tokens_override") is not None:
            kwargs["max_tokens"] = spec["_max_tokens_override"]
        if spec.get("tools"):
            kwargs["tools"] = spec["tools"]

    if spec.get("extra_body"):
        kwargs["extra_body"] = spec["extra_body"]
    if spec.get("_top_level"):
        kwargs.update(spec["_top_level"])

    return kwargs


def _write_output(
    out_dir: Path,
    scenario: str,
    model: str,
    protocol: str,
    request_messages: list[dict],
    request_kwargs: dict[str, Any],
    is_streaming: bool,
    result: dict | str,
    expect: str,
) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    is_anthropic = protocol == "anthropic"

    extra_body_sent = request_kwargs.pop("extra_body", None)
    tools_sent = request_kwargs.pop("tools", None)
    system_sent = request_kwargs.pop("system", None)
    max_tokens_sent = request_kwargs.pop("max_tokens", None)
    temperature_sent = request_kwargs.pop("temperature", None)
    top_p_sent = request_kwargs.pop("top_p", None)

    request_kwargs_clean = dict(request_kwargs)  # preserve for caller

    if is_streaming:
        out_file = out_dir / f"{scenario}.txt"
        out_file.write_text(result if isinstance(result, str) else str(result), encoding="utf-8")
        meta = {
            "protocol": protocol,
            "streaming": True,
            "scenario": scenario,
            "model": model,
            "expect": expect,
            "request": {"model": model, "messages": request_messages},
        }
        if extra_body_sent is not None:
            meta["extra_body_sent"] = extra_body_sent
        if tools_sent is not None:
            meta["tools_sent"] = tools_sent
        if max_tokens_sent is not None:
            meta["max_tokens_sent"] = max_tokens_sent
        if system_sent is not None:
            meta["system_sent"] = system_sent
        if temperature_sent is not None:
            meta["temperature_sent"] = temperature_sent
        if top_p_sent is not None:
            meta["top_p_sent"] = top_p_sent
        meta_file = out_dir / f"{scenario}-meta.json"
        meta_file.write_text(json.dumps(meta, indent=2, ensure_ascii=False), encoding="utf-8")
    else:
        output: dict[str, Any] = {
            "protocol": protocol,
            "streaming": False,
            "scenario": scenario,
            "model": model,
            "expect": expect,
            "request": {"model": model, "messages": request_messages},
            "response": result,
        }
        if extra_body_sent is not None:
            output["extra_body_sent"] = extra_body_sent
        if tools_sent is not None:
            output["tools_sent"] = tools_sent
        if max_tokens_sent is not None:
            output["max_tokens_sent"] = max_tokens_sent
        if system_sent is not None:
            output["system_sent"] = system_sent
        if temperature_sent is not None:
            output["temperature_sent"] = temperature_sent
        if top_p_sent is not None:
            output["top_p_sent"] = top_p_sent
        out_file = out_dir / f"{scenario}.json"
        out_file.write_text(json.dumps(output, indent=2, ensure_ascii=False), encoding="utf-8")


def _write_error_output(
    out_dir: Path, scenario: str, model: str, protocol: str,
    request_messages: list[dict], request_kwargs: dict[str, Any],
    expect: str, error_info: dict,
) -> None:
    """写入错误信息文件（场景不支持该模型时也走这里）。"""
    out_dir.mkdir(parents=True, exist_ok=True)

    extra_body_sent = request_kwargs.pop("extra_body", None)
    tools_sent = request_kwargs.pop("tools", None)
    system_sent = request_kwargs.pop("system", None)
    max_tokens_sent = request_kwargs.pop("max_tokens", None)

    output: dict[str, Any] = {
        "protocol": protocol,
        "streaming": False,
        "scenario": scenario,
        "model": model,
        "expect": expect,
        "request": {"model": model, "messages": request_messages},
        "response": error_info,
        "error": True,
    }
    if extra_body_sent is not None:
        output["extra_body_sent"] = extra_body_sent
    if tools_sent is not None:
        output["tools_sent"] = tools_sent
    if max_tokens_sent is not None:
        output["max_tokens_sent"] = max_tokens_sent
    if system_sent is not None:
        output["system_sent"] = system_sent

    out_file = out_dir / f"{scenario}.json"
    out_file.write_text(json.dumps(output, indent=2, ensure_ascii=False), encoding="utf-8")


# ============================================================
# 单场景采集
# ============================================================

def capture_single(
    client: LLMClient,
    scenario: str,
    model: str,
    out_dir: Path,
    api_key: str,
) -> None:
    spec = SCENARIOS[scenario]
    messages = list(spec["messages"])
    protocol = spec["protocol"]
    is_streaming = spec["expect"] in ("streaming", "tool_calls_streaming", "tool_use_streaming")
    expect = spec["expect"]
    is_openai = protocol == "openai"

    # ---- 工具多轮交互 ----
    if expect in ("tool_result",):
        _capture_tool_result(client, scenario, model, out_dir, spec, messages, protocol, expect)
        return

    # ---- Anthropic 流式 + 工具调用 ----
    if expect == "tool_use_streaming":
        _capture_anthropic_tool_streaming(client, scenario, model, out_dir, spec, messages, protocol)
        return

    # ---- GLM 流式工具调用 ----
    if scenario == "glm-tool-use-streaming":
        _capture_glm_tool_streaming(client, scenario, model, out_dir, spec, messages, protocol)
        return

    # ---- Cache 场景（增量多轮）----
    if scenario in ("cache", "anthropic-cache") and not spec.get("_context_pressure"):
        _capture_cache(client, scenario, model, out_dir, spec, messages, protocol)
        return

    # ---- Context 压力测试场景 ----
    if spec.get("_context_pressure"):
        _capture_context_pressure(client, scenario, model, out_dir, spec, messages, protocol)
        return

    # ---- 常规单次/重复请求 ----
    headers_backup: dict[str, str] | None = None
    if spec.get("_invalid_key"):
        headers_backup = dict(client.headers)
        if is_openai:
            client.headers["Authorization"] = "Bearer invalid_key_xxx"
        else:
            client.headers["x-api-key"] = "invalid_key_xxx"

    req_model = spec.get("_invalid_model", model)
    request_kwargs = _build_request_kwargs(spec, messages)

    if spec.get("_tools_invalid") is not None:
        request_kwargs["tools"] = spec["_tools_invalid"]

    repeat = spec.get("repeat", 1)
    results: list[dict] = []

    for i in range(repeat):
        try:
            if is_streaming:
                raw = client.chat_streaming(messages, req_model, **request_kwargs)
                results.append({"_raw": raw})
            else:
                result = client.chat(messages, req_model, **request_kwargs)
                results.append(result)
        except HTTPResponseError as e:
            results.append({
                "error": True,
                "http_code": e.code,
                "reason": e.reason,
                "body": e.body,
            })
        if i < repeat - 1:
            time.sleep(0.5)

    final_result = results[0] if len(results) == 1 else results

    _write_output(
        out_dir, scenario, model, protocol,
        messages, request_kwargs, is_streaming,
        final_result, expect,
    )

    if headers_backup is not None:
        client.headers = headers_backup


def _capture_tool_result(
    client: LLMClient,
    scenario: str,
    model: str,
    out_dir: Path,
    spec: dict[str, Any],
    messages: list[dict],
    protocol: str,
    expect: str,
) -> None:
    is_openai = protocol == "openai"
    mock_response = spec.get("_mock_tool_response", "fake result")
    request_kwargs = _build_request_kwargs(spec, messages)

    try:
        round1 = client.chat(messages, model, **request_kwargs)
    except HTTPResponseError as e:
        _write_error_output(out_dir, scenario, model, protocol, messages, request_kwargs, expect, {
            "error": True,
            "http_code": e.code,
            "reason": e.reason,
            "body": e.body,
        })
        return

    tool_call_info: dict | None = None
    if is_openai:
        choice = round1.get("choices", [{}])[0]
        msg = choice.get("message", {})
        tool_calls = msg.get("tool_calls", [])
        if tool_calls:
            tc = tool_calls[0]
            tool_call_info = {
                "id": tc.get("id"),
                "name": tc.get("function", {}).get("name"),
                "arguments": tc.get("function", {}).get("arguments"),
            }
    else:
        content = round1.get("content", [])
        for block in content:
            if block.get("type") == "tool_use":
                tool_call_info = {
                    "id": block.get("id"),
                    "name": block.get("name"),
                    "input": block.get("input"),
                }
                break

    round2_messages = list(messages)
    if is_openai:
        round1_msg = round1.get("choices", [{}])[0].get("message", {})
        round2_messages.append(round1_msg)
        tc = tool_call_info
        round2_messages.append({
            "role": "tool",
            "tool_call_id": tc["id"] if tc else "unknown",
            "content": mock_response,
        })
    else:
        round1_content = round1.get("content", [])
        round2_messages.append({"role": "assistant", "content": round1_content})
        if tool_call_info:
            round2_messages.append({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": tool_call_info["id"],
                    "content": mock_response,
                }],
            })

    try:
        round2 = client.chat(round2_messages, model, **request_kwargs)
    except HTTPResponseError as e:
        round2 = {
            "error": True,
            "http_code": e.code,
            "reason": e.reason,
            "body": e.body,
        }

    extra_body_sent = request_kwargs.pop("extra_body", None)
    tools_sent = request_kwargs.pop("tools", None)
    system_sent = request_kwargs.pop("system", None)
    max_tokens_sent = request_kwargs.pop("max_tokens", None)

    output: dict[str, Any] = {
        "protocol": protocol,
        "streaming": False,
        "scenario": scenario,
        "model": model,
        "expect": expect,
        "request": {"model": model, "messages": messages},
        "response": {"round1": round1, "round2": round2},
        "rounds": [
            {
                "round": 1,
                "request_messages": messages,
                "tool_call": tool_call_info,
                "mock_tool_response": mock_response,
                "response": round1,
            },
            {
                "round": 2,
                "request_messages": round2_messages,
                "response": round2,
            },
        ],
    }
    if extra_body_sent is not None:
        output["extra_body_sent"] = extra_body_sent
    if tools_sent is not None:
        output["tools_sent"] = tools_sent
    if system_sent is not None:
        output["system_sent"] = system_sent
    if max_tokens_sent is not None:
        output["max_tokens_sent"] = max_tokens_sent

    out_dir.mkdir(parents=True, exist_ok=True)
    out_file = out_dir / f"{scenario}.json"
    out_file.write_text(json.dumps(output, indent=2, ensure_ascii=False), encoding="utf-8")


def _capture_anthropic_tool_streaming(
    client: LLMClient,
    scenario: str,
    model: str,
    out_dir: Path,
    spec: dict[str, Any],
    messages: list[dict],
    protocol: str,
) -> None:
    request_kwargs = _build_request_kwargs(spec, messages)
    try:
        raw = client.chat_streaming(messages, model, **request_kwargs)
    except HTTPResponseError as e:
        raw = f"Error: HTTP {e.code} {e.reason}: {e.body}"

    extra_body_sent = request_kwargs.pop("extra_body", None)
    tools_sent = request_kwargs.pop("tools", None)
    system_sent = request_kwargs.pop("system", None)
    max_tokens_sent = request_kwargs.pop("max_tokens", None)

    out_dir.mkdir(parents=True, exist_ok=True)
    out_file = out_dir / f"{scenario}.txt"
    out_file.write_text(raw, encoding="utf-8")

    meta = {
        "protocol": protocol,
        "streaming": True,
        "scenario": scenario,
        "model": model,
        "expect": spec["expect"],
        "request": {"model": model, "messages": messages},
    }
    if extra_body_sent is not None:
        meta["extra_body_sent"] = extra_body_sent
    if tools_sent is not None:
        meta["tools_sent"] = tools_sent
    if system_sent is not None:
        meta["system_sent"] = system_sent
    if max_tokens_sent is not None:
        meta["max_tokens_sent"] = max_tokens_sent

    meta_file = out_dir / f"{scenario}-meta.json"
    meta_file.write_text(json.dumps(meta, indent=2, ensure_ascii=False), encoding="utf-8")


def _capture_glm_tool_streaming(
    client: LLMClient,
    scenario: str,
    model: str,
    out_dir: Path,
    spec: dict[str, Any],
    messages: list[dict],
    protocol: str,
) -> None:
    request_kwargs = _build_request_kwargs(spec, messages)
    try:
        raw = client.chat_streaming(messages, model, **request_kwargs)
    except HTTPResponseError as e:
        raw = f"Error: HTTP {e.code} {e.reason}: {e.body}"

    extra_body_sent = request_kwargs.pop("extra_body", None)
    tools_sent = request_kwargs.pop("tools", None)

    out_dir.mkdir(parents=True, exist_ok=True)
    out_file = out_dir / f"{scenario}.txt"
    out_file.write_text(raw, encoding="utf-8")

    meta = {
        "protocol": protocol,
        "streaming": True,
        "scenario": scenario,
        "model": model,
        "expect": spec["expect"],
        "request": {"model": model, "messages": messages},
    }
    if extra_body_sent is not None:
        meta["extra_body_sent"] = extra_body_sent
    if tools_sent is not None:
        meta["tools_sent"] = tools_sent

    meta_file = out_dir / f"{scenario}-meta.json"
    meta_file.write_text(json.dumps(meta, indent=2, ensure_ascii=False), encoding="utf-8")


def _extract_assistant_text(result: dict, is_anthropic: bool) -> str:
    """从 API 响应中提取 assistant 回复文本。"""
    if is_anthropic:
        content = result.get("content", [])
        return "\n".join(
            block.get("text", "") for block in content
            if block.get("type") == "text"
        )
    else:
        choices = result.get("choices", [])
        return choices[0].get("message", {}).get("content", "") if choices else ""


def _capture_context_pressure(
    client: LLMClient,
    scenario: str,
    model: str,
    out_dir: Path,
    spec: dict[str, Any],
    messages: list[dict],
    protocol: str,
) -> None:
    """多轮对话：每轮叠加 messages，可选 revert 回退到早期节点发新问题。

    spec 字段：
      _context_turns: 总轮数（含 revert 轮）
      _context_followups: 每轮追加的 user 消息列表（正常线性叠加）
      _revert_after_turn: 保留前 N 轮的 Q&A 后做 revert（如 =1 表示保留 Q1+A1）
      _revert_at_turn: 在第 K 轮执行 revert（1-based，默认 = _revert_after_turn + 1）
      _revert_user: revert 后发送的新 user 消息
      _revert_followups: revert 后继续追加的 user 消息列表
    """
    is_anthropic = protocol == "anthropic"
    request_kwargs = _build_request_kwargs(spec, messages)
    turns = spec.get("_context_turns", 5)
    followups = spec.get("_context_followups", [])
    revert_after_turn = spec.get("_revert_after_turn")
    revert_at_turn = spec.get("_revert_at_turn", (revert_after_turn or 0) + 1)
    revert_user = spec.get("_revert_user")
    revert_followups = spec.get("_revert_followups", [])

    system_sent = request_kwargs.pop("system", None)

    snapshots: list[dict] = []
    results: list[dict] = []
    current_messages = list(messages)
    # completed_turns[N] = 第 N 轮 Q&A 完成后的 messages 快照
    # completed_turns[0] = 初始 messages（还没有任何 Q&A）
    # completed_turns[1] = Q1+A1 完成后的 messages
    completed_turns: list[list[dict]] = [list(current_messages)]
    revert_done = False

    for i in range(turns):
        # 是否做 revert
        is_revert = (revert_after_turn is not None
                     and revert_user is not None
                     and not revert_done
                     and i + 1 == revert_at_turn)

        if is_revert:
            # revert：保留前 revert_after_turn 轮的 Q&A，丢掉之后的
            base = completed_turns[revert_after_turn]
            current_messages = [dict(m) for m in base]
            current_messages.append({"role": "user", "content": revert_user})
            revert_done = True

        # 记录本轮发送的完整 messages 快照
        snapshots.append({
            "turn": i + 1,
            "is_revert": is_revert,
            "messages": [dict(m) for m in current_messages],
        })

        round_kwargs = dict(request_kwargs)
        if is_anthropic and system_sent:
            round_kwargs["system"] = system_sent

        try:
            result = client.chat(current_messages, model, **round_kwargs)
            results.append(result)

            assistant_text = _extract_assistant_text(result, is_anthropic)
            current_messages.append({"role": "assistant", "content": assistant_text})

            # 记录本轮 Q&A 完成后的 messages
            completed_turns.append(list(current_messages))

            # 追加下一轮 user 消息
            if is_revert or (revert_done and i > 0):
                # revert 后的线性追加
                rev_idx = i - next(j for j in range(i, -1, -1)
                                   if snapshots[j].get("is_revert"))
                if rev_idx < len(revert_followups):
                    current_messages.append({"role": "user", "content": revert_followups[rev_idx]})
            elif i < len(followups):
                current_messages.append({"role": "user", "content": followups[i]})

        except HTTPResponseError as e:
            results.append({
                "error": True,
                "http_code": e.code,
                "reason": e.reason,
                "body": e.body,
            })
            break

        if i < turns - 1:
            time.sleep(1)

    output: dict[str, Any] = {
        "protocol": protocol,
        "streaming": False,
        "scenario": scenario,
        "model": model,
        "expect": spec["expect"],
        "turns": [
            {**snap, "response": results[i] if i < len(results) else None}
            for i, snap in enumerate(snapshots)
        ],
    }

    if system_sent is not None:
        output["system_sent"] = system_sent

    out_dir.mkdir(parents=True, exist_ok=True)
    out_file = out_dir / f"{scenario}.json"
    out_file.write_text(json.dumps(output, indent=2, ensure_ascii=False), encoding="utf-8")


def _capture_cache(
    client: LLMClient,
    scenario: str,
    model: str,
    out_dir: Path,
    spec: dict[str, Any],
    messages: list[dict],
    protocol: str,
) -> None:
    is_anthropic = protocol == "anthropic"
    results: list[dict] = []
    request_kwargs = _build_request_kwargs(spec, messages)

    for i in range(spec.get("repeat", 3)):
        round_kwargs = dict(request_kwargs)
        if is_anthropic:
            system = spec.get("system", round_kwargs.get("system", []))
            if system:
                system = list(system)
                last_system = dict(system[-1])
                last_system["cache_control"] = {"type": "ephemeral"}
                system[-1] = last_system
                round_kwargs["system"] = system

        try:
            result = client.chat(messages, model, **round_kwargs)
            results.append(result)
        except HTTPResponseError as e:
            results.append({
                "error": True,
                "http_code": e.code,
                "reason": e.reason,
                "body": e.body,
            })
        if i < spec.get("repeat", 3) - 1:
            time.sleep(0.5)

    extra_body_sent = request_kwargs.pop("extra_body", None)
    tools_sent = request_kwargs.pop("tools", None)
    system_sent = request_kwargs.pop("system", None)
    max_tokens_sent = request_kwargs.pop("max_tokens", None)

    output: dict[str, Any] = {
        "protocol": protocol,
        "streaming": False,
        "scenario": scenario,
        "model": model,
        "expect": spec["expect"],
        "request": {"model": model, "messages": messages},
        "response": results if len(results) != 1 else results[0],
    }
    if extra_body_sent is not None:
        output["extra_body_sent"] = extra_body_sent
    if tools_sent is not None:
        output["tools_sent"] = tools_sent
    if system_sent is not None:
        output["system_sent"] = system_sent
    if max_tokens_sent is not None:
        output["max_tokens_sent"] = max_tokens_sent
    if is_anthropic:
        output["cache_control_note"] = "Anthropic 协议在 system 末尾标记了 cache_control:ephemeral"

    out_dir.mkdir(parents=True, exist_ok=True)
    out_file = out_dir / f"{scenario}.json"
    out_file.write_text(json.dumps(output, indent=2, ensure_ascii=False), encoding="utf-8")


# ============================================================
# 非 Chat 场景采集
# ============================================================

def capture_non_chat(
    provider: str,
    api_key: str,
    scenario: str,
    out_dir: Path,
) -> None:
    """采集非 Chat 类 API（model-list, usage-quota）。"""
    cfg = PROVIDER_CONFIG[provider]
    spec = NON_CHAT_SCENARIOS[scenario]
    url_key = spec["provider_url_key"]

    # 确定 URL
    url = cfg.get(url_key)
    if not url:
        # 特殊处理：MiniMax 无 usage_url
        msg = f"{provider} 无 {scenario} API"
        print(f"    跳过: {msg}")
        _write_non_chat_skip(out_dir, scenario, provider, msg)
        return

    # model-list 对 Anthropic 协议可能有独立 URL
    if scenario == "model-list":
        anthropic_url_key = spec.get("anthropic_url_key")
        if anthropic_url_key and cfg.get(anthropic_url_key):
            url = cfg[anthropic_url_key]  # 优先使用 anthropic 专用 URL

    headers = {"Authorization": f"Bearer {api_key}"}

    print(f"    GET {url}", end=" ", flush=True)
    try:
        result = _http_get(url, headers)
        print("✓")
    except HTTPResponseError as e:
        result = {
            "error": True,
            "http_code": e.code,
            "reason": e.reason,
            "body": e.body,
        }
        print(f"✗ HTTP {e.code}")
    except Exception as e:
        result = {"error": True, "reason": str(e)}
        print(f"✗ {e}")

    output: dict[str, Any] = {
        "protocol": "n/a",
        "streaming": False,
        "scenario": scenario,
        "model": "provider",
        "provider": provider,
        "expect": scenario,
        "request": {"method": "GET", "url": url},
        "response": result,
    }

    out_dir.mkdir(parents=True, exist_ok=True)
    out_file = out_dir / f"{scenario}.json"
    out_file.write_text(json.dumps(output, indent=2, ensure_ascii=False), encoding="utf-8")


def _write_non_chat_skip(
    out_dir: Path, scenario: str, provider: str, message: str
) -> None:
    """写入跳过信息文件（provider 不支持该非 Chat 场景）。"""
    output: dict[str, Any] = {
        "protocol": "n/a",
        "streaming": False,
        "scenario": scenario,
        "model": "provider",
        "provider": provider,
        "expect": scenario,
        "request": {},
        "response": {"skipped": True, "reason": message},
    }
    out_dir.mkdir(parents=True, exist_ok=True)
    out_file = out_dir / f"{scenario}.json"
    out_file.write_text(json.dumps(output, indent=2, ensure_ascii=False), encoding="utf-8")


# ============================================================
# 主入口
# ============================================================

def main() -> None:
    parser = argparse.ArgumentParser(
        description="LLM Fixture 采集 — provider + model + protocol → 跑全部适用场景"
    )
    parser.add_argument("--provider", required=True, choices=list(PROVIDER_CONFIG.keys()))
    parser.add_argument("--model", required=True, help="模型名，如 MiniMax-M2.7 / glm-5.1 / deepseek-v4-flash")
    parser.add_argument("--protocol", required=True, choices=["openai", "anthropic"])
    parser.add_argument("--api-key", default=os.environ.get("LLM_API_KEY", ""))
    parser.add_argument(
        "--output-base",
        default="/home/admin/code/closeclaw-test/tests/fixtures/llm/v2",
    )
    parser.add_argument(
        "--scenario-type",
        choices=["chat", "non-chat", "all"],
        default="all",
        help="采集场景类型: chat=仅对话, non-chat=仅非对话(model-list/usage-quota), all=全部",
    )
    args = parser.parse_args()

    if not args.api_key:
        print("Error: --api-key 或环境变量 LLM_API_KEY 未设置", file=sys.stderr)
        sys.exit(1)

    output_base = Path(args.output_base)
    stype = args.scenario_type

    done = 0
    fail = 0

    # ---- Chat 场景 ----
    if stype in ("chat", "all"):
        out_dir = output_base / args.provider / args.model / args.protocol

        # 清理该组合下的旧文件
        if out_dir.exists():
            for f in out_dir.iterdir():
                f.unlink()

        client = make_client(args.provider, args.api_key, args.protocol)

        # 筛选出适用于该 (provider, protocol) 的所有场景
        applicable = [
            name for name, spec in SCENARIOS.items()
            if spec["protocol"] == args.protocol
            and (spec.get("provider") is None or spec["provider"] == args.provider)
        ]

        print(f"[{args.provider}/{args.model}/{args.protocol}] 共 {len(applicable)} 个 chat 场景")
        print(f"输出目录: {out_dir}")
        print("-" * 60)

        for scenario in sorted(applicable):
            print(f"  {scenario} ...", end=" ", flush=True)
            try:
                capture_single(client, scenario, args.model, out_dir, args.api_key)
                print("✓")
                done += 1
            except Exception as e:
                print(f"✗ {e}")
                fail += 1

    # ---- 非 Chat 场景 ----
    if stype in ("non-chat", "all"):
        provider_out_dir = output_base / args.provider / "provider"

        print(f"")
        print(f"[{args.provider}/provider] 非 Chat 场景")
        print(f"输出目录: {provider_out_dir}")
        print("-" * 60)

        for scenario in sorted(NON_CHAT_SCENARIOS.keys()):
            print(f"  {scenario} ...", end=" ", flush=True)
            try:
                capture_non_chat(
                    args.provider, args.api_key, scenario, provider_out_dir
                )
                done += 1
            except Exception as e:
                print(f"✗ {e}")
                fail += 1

    print("-" * 60)
    print(f"完成: {done} 成功, {fail} 失败")


if __name__ == "__main__":
    main()
