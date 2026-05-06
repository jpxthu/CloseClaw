#!/usr/bin/env python3
"""
LLM Provider Fixture 采集脚本 — v2

基于 Phase 0 深挖文档重建：
- 所有字段和参数必须出现在 Phase 0 文档中
- Phase 0 未覆盖的字段全部删除
- 场景定义与文档覆盖情况一一对应

输出目录结构：
  tests/fixtures/llm/v2/{provider}/{protocol}/
  例如：tests/fixtures/llm/v2/minimax/openai/
       tests/fixtures/llm/v2/minimax/anthropic/

Fixture 顶层字段（仅文档有记录的字段）：
  protocol         - "openai" | "anthropic"
  streaming        - true | false
  scenario         - 场景名
  model            - 模型名
  expect           - 期望结果类型
  request          - 发送的请求体（仅含文档字段）
  extra_body_sent  - 额外发送的参数（仅文档有记录的 extra_body 字段）
  response         - 响应体（JSON 或流式原始文本）
"""


from abc import ABC, abstractmethod
from pathlib import Path
from typing import Any

import argparse
import json
import os
import sys
import time

import urllib.request
import urllib.error

from providers import PROVIDER_CONFIG


# ============================================================
# HTTP 底层
# ============================================================

class HTTPResponseError(Exception):
    """HTTP 错误：MiniMax 业务错误走 HTTP 200 + base_resp，
    其他 provider 走标准 4xx。"""

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
    """OpenAI 兼容协议客户端。"""

    def __init__(
        self,
        base_url: str,
        api_key: str,
        error_via_base_resp: bool = False,
        extra_headers: dict | None = None,
    ):
        self.base_url = base_url.rstrip("/")
        self._error_via_base_resp = error_via_base_resp
        self.headers = {
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
            **(extra_headers or {}),
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
    """Anthropic 兼容协议客户端。

    请求字段（Phase 0 文档有记录）：
      model, messages, system（独立字段，非 role）,
      max_tokens, stream, temperature, top_p

    Anthropic 协议不支持 system 作为 messages 中的 role。
    """

    def __init__(
        self,
        base_url: str,
        api_key: str,
        extra_headers: dict | None = None,
    ):
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.headers = {
            "x-api-key": api_key,
            "anthropic-version": "2023-06-01",
            "Content-Type": "application/json",
            **(extra_headers or {}),
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
# 共享工具定义（文档标准格式）
# ============================================================

# OpenAI 协议 tools 格式
OPENAI_TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "get_weather",
            "description": "Get the current weather in a given location.",
            "parameters": {
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "City name, e.g. Tokyo",
                    }
                },
                "required": ["location"],
            },
        },
    }
]

# Anthropic 协议 tools 格式（注意：name 而非 function.name，input_schema 而非 function.parameters）
ANTHROPIC_TOOLS = [
    {
        "name": "get_weather",
        "description": "Get the current weather in a given location.",
        "input_schema": {
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "City name, e.g. Tokyo",
                }
            },
            "required": ["location"],
        },
    }
]


# ============================================================
# 场景定义
#
# 字段来源规则：
#   - 请求参数字段名必须出现在 Phase 0 文档的请求参数表
#   - 响应字段名必须出现在 Phase 0 文档的响应格式章节
#   - Phase 0 文档"未明确说明"的字段，不写入脚本
# ============================================================

SCENARIOS: dict[str, dict[str, Any]] = {
    # ---------- OpenAI: 基础 single-turn ----------
    "simple": {
        "protocol": "openai",
        "description": "基础 single-turn 对话，验证 OpenAI 协议响应格式",
        "messages": [{"role": "user", "content": "Say hello in 3 words."}],
        "expect": "text",
    },

    # ---------- OpenAI: Streaming ----------
    "streaming": {
        "protocol": "openai",
        "description": "SSE stream，验证 chunk 格式（finish_reason / delta.role）",
        "messages": [{"role": "user", "content": "Count from 1 to 3."}],
        "expect": "streaming",
    },

    # ---------- OpenAI: 多轮对话 ----------
    "multi-turn": {
        "protocol": "openai",
        "description": "多轮对话，上下文通过 messages 历史传递",
        "messages": [
            {"role": "user", "content": "My name is Alice."},
            {"role": "assistant", "content": "Hi Alice! Nice to meet you."},
            {"role": "user", "content": "What is my name?"},
        ],
        "expect": "context",
    },

    # ---------- OpenAI: Cache ----------
    # MiniMax 主动缓存需在 system 末尾使用 cache_control 标记断点
    # Anthropic 协议：system 数组中最后一个块加 cache_control
    # OpenAI 协议：MiniMax 不直接支持 cache_control，需通过 anthropic_url + cache_control
    # 这里使用 Anthropic 协议的 cache 场景
    "cache": {
        "protocol": "openai",
        "description": "验证 OpenAI 协议下 prompt_tokens_details.cached_tokens 字段",
        "messages": [
            {
                "role": "system",
                "content": "You are a helpful assistant. Always remember: the sky is blue.",
            },
            {"role": "user", "content": "What color is the sky?"},
        ],
        "expect": "cache",
        "repeat": 3,
    },

    # ---------- OpenAI: Reasoning / Thinking ----------
    # MiniMax: reasoning_split（extra_body）—— content 仍含 ‖标签，reasoning_details 数组独立出现
    "minimax-reasoning-split": {
        "protocol": "openai",
        "provider": "minimax",
        "description": "MiniMax reasoning_split=true，验证 reasoning_details 字段",
        "messages": [
            {"role": "user", "content": "What is 17*23? Show your work step by step."}
        ],
        "expect": "reasoning",
        "extra_body": {"reasoning_split": True},
    },

    # GLM: thinking（extra_body）—— reasoning_content 字段
    "glm-thinking": {
        "protocol": "openai",
        "provider": "glm",
        "description": "GLM thinking enabled，验证 choices[].message.reasoning_content",
        "messages": [
            {"role": "user", "content": "What is 17*23? Show your work step by step."}
        ],
        "expect": "reasoning",
        "extra_body": {"thinking": {"type": "enabled"}},
    },

    # DeepSeek: reasoning_effort（顶层字段）—— reasoning_content 字段
    "deepseek-thinking-high": {
        "protocol": "openai",
        "provider": "deepseek",
        "description": "DeepSeek reasoning_effort=high，验证 choices[].message.reasoning_content",
        "messages": [
            {"role": "user", "content": "What is 17*23? Show your work step by step."}
        ],
        "expect": "reasoning",
        "_top_level": {"reasoning_effort": "high"},
    },

    "deepseek-thinking-disabled": {
        "protocol": "openai",
        "provider": "deepseek",
        "description": "DeepSeek thinking disabled，验证无 reasoning_content",
        "messages": [
            {"role": "user", "content": "What is 17*23?"}
        ],
        "expect": "no-reasoning",
        "_top_level": {"reasoning_effort": "low"},
    },

    # ---------- OpenAI: 工具调用（Tool Use）----------
    # 第一步：触发工具调用
    "tool-use": {
        "protocol": "openai",
        "description": "工具调用触发，验证 finish_reason=tool_calls 和 tool_calls 格式",
        "messages": [
            {
                "role": "user",
                "content": "How's the weather in San Francisco?",
            }
        ],
        "expect": "tool_calls",
        "tools": OPENAI_TOOLS,
    },

    # 第二步：工具结果回传 + 获取最终回复（多轮交互）
    "tool-result": {
        "protocol": "openai",
        "description": "完整工具调用多轮交互：call → tool_result → final_response",
        "messages": [
            {
                "role": "user",
                "content": "How's the weather in San Francisco?",
            }
        ],
        "expect": "tool_result",
        "tools": OPENAI_TOOLS,
        "extra_body": {"reasoning_split": True},
        "_mock_tool_response": "24℃, sunny",
    },

    # Streaming + 工具调用
    "tool-use-streaming": {
        "protocol": "openai",
        "description": "Streaming 下工具调用，验证 SSE chunk 中 tool_calls delta 格式",
        "messages": [
            {
                "role": "user",
                "content": "How's the weather in San Francisco?",
            }
        ],
        "expect": "tool_calls_streaming",
        "tools": OPENAI_TOOLS,
    },

    # ---------- OpenAI: 错误场景 ----------
    "error-auth": {
        "protocol": "openai",
        "description": "无效 API key，验证错误格式",
        "messages": [{"role": "user", "content": "hi"}],
        "expect": "error",
        "_invalid_key": True,
    },

    "error-model": {
        "protocol": "openai",
        "description": "无效 model name",
        "messages": [{"role": "user", "content": "hi"}],
        "expect": "error",
        "_invalid_model": "this-model-does-not-exist-999",
    },

    "error-empty": {
        "protocol": "openai",
        "description": "空 messages，参数错误",
        "messages": [],
        "expect": "error",
    },

    # ---------- Anthropic: 通用 ----------
    "anthropic-simple": {
        "protocol": "anthropic",
        "description": "Anthropic 协议基础请求，验证响应 content[].type 和 usage 字段",
        "messages": [{"role": "user", "content": "Say hello in 3 words."}],
        "expect": "text",
        "max_tokens": 1024,
    },

    # ---------- Anthropic: Thinking block ----------
    # MiniMax: content[].type=="thinking" + thinking/signature
    # 模型默认输出 thinking，thinking 参数实际被忽略
    "anthropic-thinking": {
        "protocol": "anthropic",
        "provider": "minimax",
        "description": "MiniMax Anthropic 端点 thinking block（默认出现，thinking 参数被忽略）",
        "messages": [
            {"role": "user", "content": "What is 17*23? Show your work step by step."}
        ],
        "expect": "reasoning",
        "max_tokens": 2048,
    },

    # ---------- Anthropic: Streaming ----------
    "anthropic-streaming": {
        "protocol": "anthropic",
        "description": "Anthropic SSE stream，验证事件序列（message_start / content_block_* / message_delta）",
        "messages": [{"role": "user", "content": "Count from 1 to 3."}],
        "expect": "streaming",
        "max_tokens": 1024,
    },

    # ---------- Anthropic: 工具调用（Tool Use）----------
    # 第一步：触发工具调用
    "anthropic-tool-use": {
        "protocol": "anthropic",
        "provider": "minimax",
        "description": "Anthropic 协议工具调用，验证 content[].type=tool_use 和 id/name/input 字段",
        "messages": [
            {
                "role": "user",
                "content": "How's the weather in San Francisco?",
            }
        ],
        "expect": "tool_use",
        "tools": ANTHROPIC_TOOLS,
        "max_tokens": 2048,
    },

    # 第二步：完整工具调用多轮交互
    "anthropic-tool-result": {
        "protocol": "anthropic",
        "provider": "minimax",
        "description": "Anthropic 完整工具调用多轮：call → tool_result → final_response",
        "messages": [
            {
                "role": "user",
                "content": "How's the weather in San Francisco?",
            }
        ],
        "expect": "tool_result",
        "tools": ANTHROPIC_TOOLS,
        "max_tokens": 2048,
        "_mock_tool_response": "24℃, sunny",
    },

    # ---------- Anthropic: Cache ----------
    # 使用 cache_control 标记 system 断点
    "anthropic-cache": {
        "protocol": "anthropic",
        "provider": "minimax",
        "description": "Anthropic cache_control 主动缓存，验证 cache_creation / cache_read 字段",
        "messages": [
            {"role": "user", "content": "What color is the sky?"}
        ],
        "expect": "cache",
        "system": [
            {"type": "text", "text": "You are a helpful assistant. Always remember: the sky is blue."},
        ],
        "repeat": 3,
        "max_tokens": 1024,
    },

    # ---------- Anthropic: 错误场景 ----------
    "anthropic-error-auth": {
        "protocol": "anthropic",
        "description": "Anthropic 端点无效 key",
        "messages": [{"role": "user", "content": "hi"}],
        "expect": "error",
        "max_tokens": 1024,
        "_invalid_key": True,
    },

    "anthropic-error-empty": {
        "protocol": "anthropic",
        "description": "Anthropic 端点空 messages",
        "messages": [],
        "expect": "error",
        "max_tokens": 1024,
    },
}


# ============================================================
# 采集逻辑
# ============================================================

def _build_request_kwargs(
    spec: dict[str, Any],
    model: str,
    messages: list[dict],
) -> dict[str, Any]:
    """构造 API 请求关键字参数（全部通过 **kwargs 传给 client.chat()）。

    返回的 dict 包含所有请求体字段：
      - model, messages
      - OpenAI 特有：temperature, top_p, tools, extra_body
      - Anthropic 特有：max_tokens, system, temperature, top_p
      - 顶层字段（如 DeepSeek reasoning_effort）
    """
    # model 和 messages 由 caller 以 positional 参数传入，kwargs 仅含可选字段
    kwargs: dict[str, Any] = {}

    # Anthropic 协议特有字段（独立顶层字段，非 extra_body）
    if spec["protocol"] == "anthropic":
        for field in ("max_tokens", "system", "temperature", "top_p"):
            if field in spec:
                kwargs[field] = spec[field]
        # Anthropic tools 格式（name + input_schema）
        if spec.get("tools"):
            kwargs["tools"] = spec["tools"]

    # OpenAI 协议特有字段
    if spec["protocol"] == "openai":
        for field in ("temperature", "top_p"):
            if field in spec:
                kwargs[field] = spec[field]
        # OpenAI tools 格式（type: "function" + function.name/parameters）
        if spec.get("tools"):
            kwargs["tools"] = spec["tools"]

    # extra_body（仅文档确认的字段，如 MiniMax reasoning_split）
    if spec.get("extra_body"):
        kwargs["extra_body"] = spec["extra_body"]

    # 顶层字段（如 DeepSeek reasoning_effort）
    if spec.get("_top_level"):
        kwargs.update(spec["_top_level"])

    return kwargs


def _capture_single(
    client: LLMClient,
    messages: list[dict],
    model: str,
    request_kwargs: dict[str, Any],
    is_streaming: bool,
) -> dict | str:
    """执行单次 API 调用（流式或非流式）。"""
    if is_streaming:
        return client.chat_streaming(messages, model, **request_kwargs)
    else:
        return client.chat(messages, model, **request_kwargs)


def _write_output(
    output_base: Path,
    protocol: str,
    scenario: str,
    model: str,
    request_messages: list[dict],
    request_kwargs: dict[str, Any],
    is_streaming: bool,
    result: dict | str,
    expect: str,
) -> Path:
    """将采集结果写入文件。"""
    out_dir = output_base / protocol
    out_dir.mkdir(parents=True, exist_ok=True)

    # 提取 extra_body_sent（仅记录文档有定义的字段）
    extra_body_sent = request_kwargs.pop("extra_body", None)
    # 提取 tools（从 kwargs 取出，不放入 meta）
    tools_sent = request_kwargs.pop("tools", None)
    # 提取 system（Anthropic 协议）
    system_sent = request_kwargs.pop("system", None)
    # 提取 max_tokens（Anthropic）
    max_tokens_sent = request_kwargs.pop("max_tokens", None)
    # 提取 temperature/top_p
    temperature_sent = request_kwargs.pop("temperature", None)
    top_p_sent = request_kwargs.pop("top_p", None)

    if is_streaming:
        raw = result  # str
        out_file = out_dir / f"{model}-{scenario}.txt"
        out_file.write_text(raw, encoding="utf-8")

        meta = {
            "protocol": protocol,
            "streaming": True,
            "scenario": scenario,
            "model": model,
            "expect": expect,
            "request": {"model": model, "messages": request_messages},
            "extra_body_sent": extra_body_sent,
        }
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

        meta_file = out_dir / f"{model}-{scenario}-meta.json"
        meta_file.write_text(
            json.dumps(meta, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        print(f"  ✓ {protocol}/streaming → {out_file.name}")
        return out_file
    else:
        output: dict[str, Any] = {
            "protocol": protocol,
            "streaming": False,
            "scenario": scenario,
            "model": model,
            "expect": expect,
            "request": {"model": model, "messages": request_messages},
            "extra_body_sent": extra_body_sent,
            "response": result,
        }
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

        out_file = out_dir / f"{model}-{scenario}.json"
        out_file.write_text(
            json.dumps(output, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        print(f"  ✓ {protocol}/{scenario} → {out_file.name}")
        return out_file


def capture(
    client: LLMClient,
    scenario: str,
    model: str,
    output_base: Path,
    api_key: str,
) -> Path:
    spec = SCENARIOS[scenario]
    messages = list(spec["messages"])  # 深拷贝，避免修改原始定义
    protocol = spec["protocol"]
    is_streaming = spec["expect"] in ("streaming", "tool_calls_streaming")
    expect = spec["expect"]

    # ---------- 特殊场景处理 ----------

    # 1. 工具多轮交互场景（tool-result / anthropic-tool-result）
    if expect in ("tool_result",):
        return _capture_tool_result(
            client, scenario, model, output_base, spec, messages, protocol, expect
        )

    # 2. 无效 key 场景：临时替换 header
    headers_backup: dict[str, str] | None = None
    if spec.get("_invalid_key"):
        headers_backup = dict(client.headers)
        if protocol == "openai":
            client.headers["Authorization"] = "Bearer invalid_key_xxx"
        else:
            client.headers["x-api-key"] = "invalid_key_xxx"

    req_model = spec.get("_invalid_model", model)

    try:
        request_kwargs = _build_request_kwargs(spec, req_model, messages)

        # ---------- Cache 场景：需要在 system 末尾加 cache_control ----------
        if scenario in ("cache", "anthropic-cache"):
            return _capture_cache(
                client, scenario, model, output_base, spec, messages, protocol,
                request_kwargs
            )

        # ---------- 常规单次或重复请求 ----------
        repeat = spec.get("repeat", 1)
        results: list[dict] = []

        for i in range(repeat):
            try:
                result = _capture_single(
                    client, messages, req_model, request_kwargs, is_streaming
                )
                results.append(result)  # type: ignore
            except HTTPResponseError as e:
                results.append(
                    {
                        "error": True,
                        "http_code": e.code,
                        "reason": e.reason,
                        "body": e.body,
                    }
                )
            if i < repeat - 1:
                time.sleep(0.5)

        final_result = results[0] if len(results) == 1 else results  # type: ignore

        return _write_output(
            output_base, protocol, scenario, req_model,
            messages, request_kwargs, is_streaming,
            final_result, expect,
        )

    finally:
        if headers_backup is not None:
            client.headers = headers_backup


def _capture_tool_result(
    client: LLMClient,
    scenario: str,
    model: str,
    output_base: Path,
    spec: dict[str, Any],
    messages: list[dict],
    protocol: str,
    expect: str,
) -> Path:
    """工具调用多轮交互采集：
    1. 发送带 tools 的请求，触发工具调用
    2. 解析 tool_call / tool_use，获取 id 和参数
    3. 构造 tool_result 消息并回传
    4. 发送第二轮请求，获取最终回复

    文档参考：
    - OpenAI: messages.append(response_message) → messages.append({role:"tool", tool_call_id, content})
    - Anthropic: messages.append({role:"assistant", content: response.content})
              → messages.append({role:"user", content: [{type:"tool_result", tool_use_id, content}]})
    """
    is_openai = protocol == "openai"
    mock_response = spec.get("_mock_tool_response", "fake result")
    request_kwargs = _build_request_kwargs(spec, model, messages)

    print(f"  → Round 1: 发送工具调用请求...")

    # ---- Round 1: 触发工具调用 ----
    try:
        round1 = client.chat(messages, model, **request_kwargs)
    except HTTPResponseError as e:
        # 如果直接返回错误，记录并退出
        out_dir = output_base / protocol
        out_dir.mkdir(parents=True, exist_ok=True)
        output: dict[str, Any] = {
            "protocol": protocol,
            "streaming": False,
            "scenario": scenario,
            "model": model,
            "expect": spec["expect"],
            "request": {"model": model, "messages": messages},
            "extra_body_sent": spec.get("extra_body"),
            "tools_sent": spec.get("tools"),
            "response": {
                "error": True,
                "http_code": e.code,
                "reason": e.reason,
                "body": e.body,
            },
            "rounds": [],
        }
        out_file = out_dir / f"{model}-{scenario}.json"
        out_file.write_text(
            json.dumps(output, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        print(f"  ✓ {protocol}/{scenario} → {out_file.name} (error)")
        return out_file

    # ---- 解析 Round 1 的工具调用 ----
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
            finish_reason = choice.get("finish_reason")
            reasoning_details = msg.get("reasoning_details")
    else:
        # Anthropic: content 数组中找 tool_use 块
        content = round1.get("content", [])
        for block in content:
            if block.get("type") == "tool_use":
                tool_call_info = {
                    "id": block.get("id"),
                    "name": block.get("name"),
                    "input": block.get("input"),
                }
                break
        stop_reason = round1.get("stop_reason")

    print(f"  → Round 1 响应: finish_reason={choice.get('finish_reason') if is_openai else stop_reason}")
    if tool_call_info:
        print(f"  → 解析到工具调用: {tool_call_info.get('name')}({tool_call_info.get('arguments') or tool_call_info.get('input')})")
    else:
        print(f"  → 未触发工具调用（模型直接返回文本）")

    # ---- 构造 Round 2 消息 ----
    round2_messages = list(messages)

    if is_openai:
        # OpenAI: 追加完整的 assistant 消息（含 tool_calls）
        round1_msg = round1.get("choices", [{}])[0].get("message", {})
        round2_messages.append(round1_msg)
        # 追加 tool result
        tc = tool_call_info
        round2_messages.append({
            "role": "tool",
            "tool_call_id": tc["id"] if tc else "unknown",
            "content": mock_response,
        })
    else:
        # Anthropic: 追加 {role:"assistant", content: [...blocks...]}
        round1_content = round1.get("content", [])
        round2_messages.append({
            "role": "assistant",
            "content": round1_content,
        })
        # 追加 tool_result（注意是 type:"tool_result"，不是 tool_use_id）
        if tool_call_info:
            round2_messages.append({
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": tool_call_info["id"],
                        "content": mock_response,
                    }
                ],
            })

    print(f"  → Round 2: 回传 tool_result，获取最终回复...")

    # ---- Round 2: 获取最终回复 ----
    try:
        round2 = client.chat(round2_messages, model, **request_kwargs)
    except HTTPResponseError as e:
        round2 = {
            "error": True,
            "http_code": e.code,
            "reason": e.reason,
            "body": e.body,
        }

    # ---- 写入文件 ----
    out_dir = output_base / protocol
    out_dir.mkdir(parents=True, exist_ok=True)

    # 提取 extra_body_sent
    extra_body_sent = request_kwargs.pop("extra_body", None)
    tools_sent = request_kwargs.pop("tools", None)
    system_sent = request_kwargs.pop("system", None)
    max_tokens_sent = request_kwargs.pop("max_tokens", None)

    output = {
        "protocol": protocol,
        "streaming": False,
        "scenario": scenario,
        "model": model,
        "expect": expect,
        "request": {"model": model, "messages": messages},
        "extra_body_sent": extra_body_sent,
        "tools_sent": tools_sent,
        "response": {
            "round1": round1,
            "round2": round2,
        },
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
    if system_sent is not None:
        output["system_sent"] = system_sent
    if max_tokens_sent is not None:
        output["max_tokens_sent"] = max_tokens_sent

    out_file = out_dir / f"{model}-{scenario}.json"
    out_file.write_text(
        json.dumps(output, indent=2, ensure_ascii=False), encoding="utf-8"
    )
    print(f"  ✓ {protocol}/{scenario} → {out_file.name}")
    return out_file


def _capture_cache(
    client: LLMClient,
    scenario: str,
    model: str,
    output_base: Path,
    spec: dict[str, Any],
    messages: list[dict],
    protocol: str,
    request_kwargs: dict[str, Any],
) -> Path:
    """Cache 场景采集：
    - OpenAI: 无显式 cache_control，仅验证 prompt_tokens_details.cached_tokens 字段
    - Anthropic: 在 system 末尾添加 cache_control:{"type":"ephemeral"} 标记断点
    """
    is_anthropic = protocol == "anthropic"
    results: list[dict] = []

    for i in range(spec.get("repeat", 3)):
        round_kwargs = dict(request_kwargs)

        if is_anthropic:
            # Anthropic: 在 system 最后一块加 cache_control
            system = spec.get("system", round_kwargs.get("system", []))
            if system:
                system = list(system)
                # 给最后一块加 cache_control
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

    # 提取 sent 字段
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
        "extra_body_sent": extra_body_sent,
        "response": results if len(results) != 1 else results[0],
    }
    if tools_sent is not None:
        output["tools_sent"] = tools_sent
    if system_sent is not None:
        output["system_sent"] = system_sent
    if max_tokens_sent is not None:
        output["max_tokens_sent"] = max_tokens_sent
    # 标注 Anthropic 缓存专用字段
    if is_anthropic:
        output["cache_control_note"] = "Anthropic 协议在 system 末尾标记了 cache_control:ephemeral"

    out_dir = output_base / protocol
    out_dir.mkdir(parents=True, exist_ok=True)
    out_file = out_dir / f"{model}-{scenario}.json"
    out_file.write_text(
        json.dumps(output, indent=2, ensure_ascii=False), encoding="utf-8"
    )
    print(f"  ✓ {protocol}/{scenario} → {out_file.name}")
    return out_file


# ============================================================
# 主入口
# ============================================================

def main() -> None:
    parser = argparse.ArgumentParser(
        description="LLM Provider Fixture 采集脚本（v2，基于 Phase 0 文档）"
    )
    parser.add_argument("--provider", required=True, choices=list(PROVIDER_CONFIG.keys()))
    parser.add_argument("--model", required=True)
    parser.add_argument(
        "--scenario",
        required=True,
        help=f"可用场景: {', '.join(sorted(SCENARIOS.keys()))}",
    )
    parser.add_argument(
        "--api-key",
        default=os.environ.get("LLM_API_KEY", ""),
    )
    parser.add_argument(
        "--output-base",
        default="tests/fixtures/llm/v2",
        help="默认: tests/fixtures/llm/v2",
    )
    args = parser.parse_args()

    if not args.api_key:
        print(
            "Error: --api-key 或环境变量 LLM_API_KEY 未设置",
            file=sys.stderr,
        )
        sys.exit(1)

    spec = SCENARIOS.get(args.scenario)
    if not spec:
        raise ValueError(f"Unknown scenario: {args.scenario}")

    protocol = spec["protocol"]
    provider = spec.get("provider", args.provider)

    output_base = Path(args.output_base) / provider
    client = make_client(provider, args.api_key, protocol)

    print(f"[{provider}/{protocol}] {args.model} × {args.scenario}")
    try:
        out_path = capture(
            client, args.scenario, args.model, output_base, args.api_key
        )
        print(f"Done: {out_path}")
    except HTTPResponseError as e:
        print(f"HTTP Error {e.code} {e.reason}: {e.body}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
