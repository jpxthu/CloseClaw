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
    # MiniMax / GLM / DeepSeek 均使用 prompt_tokens_details.cached_tokens
    "cache": {
        "protocol": "openai",
        "description": "多轮重复请求，验证 cached_tokens 字段出现",
        "messages": [
            {
                "role": "system",
                "content": "You are a helpful assistant. Always remember: the sky is blue.",
            },
            {"role": "user", "content": "What color is the sky?"},
        ],
        "expect": "cache",
        "repeat": 3,  # 首次写入缓存，后续命中 cached_tokens 应 > 0
    },

    # ---------- OpenAI: Reasoning / Thinking ----------
    # 各 provider 参数名不同，场景独立定义

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
    "tool-use": {
        "protocol": "openai",
        "description": "工具调用，验证 tool_calls 响应格式和 finish_reason=tool_calls",
        "messages": [
            {
                "role": "user",
                "content": "What is the weather in Tokyo?",
            }
        ],
        "expect": "tool_calls",
        "tools": [
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
        ],
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
    # ⚠️ GLM Anthropic 端点路径未文档化，基于标准路径反推，待实测
    "anthropic-simple": {
        "protocol": "anthropic",
        "description": "Anthropic 协议基础请求，验证响应 content[].type 和 usage 字段 【⚠️待实测-GLM】",
        "messages": [{"role": "user", "content": "Say hello in 3 words."}],
        "expect": "text",
        "max_tokens": 1024,
    },

    # ---------- Anthropic: Thinking block ----------
    # MiniMax: content[].type=="thinking" + thinking/signature
    # ⚠️ GLM: 文档无 Anthropic 端点响应格式记录，【⚠️待实测】
    "anthropic-thinking": {
        "protocol": "anthropic",
        "provider": "minimax",
        "description": "MiniMax Anthropic 端点 thinking block 【⚠️待实测-GLM】",
        "messages": [
            {"role": "user", "content": "What is 17*23? Show your work step by step."}
        ],
        "expect": "reasoning",
        "max_tokens": 2048,
    },

    # ---------- Anthropic: Streaming ----------
    # ⚠️ GLM: 文档无 Anthropic SSE 事件序列记录，【⚠️待实测】
    "anthropic-streaming": {
        "protocol": "anthropic",
        "description": "Anthropic SSE stream，验证事件序列 【⚠️待实测-GLM】",
        "messages": [{"role": "user", "content": "Count from 1 to 3."}],
        "expect": "streaming",
        "max_tokens": 1024,
    },

    # ---------- Anthropic: 错误场景 ----------
    "anthropic-error-auth": {
        "protocol": "anthropic",
        "description": "Anthropic 端点无效 key 【⚠️待实测-GLM】",
        "messages": [{"role": "user", "content": "hi"}],
        "expect": "error",
        "max_tokens": 1024,
        "_invalid_key": True,
    },

    "anthropic-error-empty": {
        "protocol": "anthropic",
        "description": "Anthropic 端点空 messages 【⚠️待实测-GLM】",
        "messages": [],
        "expect": "error",
        "max_tokens": 1024,
    },
}


# ============================================================
# 采集逻辑
# ============================================================

def _build_payload(
    spec: dict[str, Any],
    model: str,
    messages: list[dict],
) -> tuple[dict[str, Any], dict[str, Any]]:
    """构造请求体。

    返回 (payload, http_kwargs)：
      payload  → 发送给 API 的请求体
      http_kwargs → chat()/chat_streaming() 的关键字参数（stream 等）
    """
    payload: dict[str, Any] = {
        "model": model,
        "messages": messages,
    }

    # Anthropic 协议特有字段
    if spec["protocol"] == "anthropic":
        for field in ("max_tokens", "system", "temperature", "top_p"):
            if field in spec:
                payload[field] = spec[field]

    # OpenAI 协议特有字段
    if spec["protocol"] == "openai":
        for field in ("temperature", "top_p"):
            if field in spec:
                payload[field] = spec[field]

    # extra_body（仅文档确认的字段）
    if spec.get("extra_body"):
        payload["extra_body"] = spec["extra_body"]

    # 顶层字段（如 DeepSeek reasoning_effort）
    if spec.get("_top_level"):
        payload.update(spec["_top_level"])

    # tools
    if spec.get("tools"):
        payload["tools"] = spec["tools"]

    http_kwargs: dict[str, Any] = {}
    return payload, http_kwargs


def capture(
    client: LLMClient,
    scenario: str,
    model: str,
    output_base: Path,
    api_key: str,
) -> Path:
    spec = SCENARIOS[scenario]
    messages = spec["messages"]
    protocol = spec["protocol"]
    is_streaming = spec["expect"] == "streaming"

    # 无效 key 场景：临时替换 header
    headers_backup: dict[str, str] | None = None
    if spec.get("_invalid_key"):
        headers_backup = dict(client.headers)
        if protocol == "openai":
            client.headers["Authorization"] = "Bearer invalid_key_xxx"
        else:
            client.headers["x-api-key"] = "invalid_key_xxx"

    req_model = spec.get("_invalid_model", model)

    try:
        payload, http_kwargs = _build_payload(spec, req_model, messages)

        if is_streaming:
            raw = client.chat_streaming(messages, req_model, **http_kwargs)
            out_dir = output_base / protocol
            out_dir.mkdir(parents=True, exist_ok=True)
            out_file = out_dir / f"{model}-{scenario}.txt"
            out_file.write_text(raw, encoding="utf-8")

            meta = {
                "protocol": protocol,
                "streaming": True,
                "scenario": scenario,
                "model": req_model,
                "expect": spec["expect"],
                "request": {"model": req_model, "messages": messages},
                "extra_body_sent": spec.get("extra_body", {}),
            }
            meta_file = out_dir / f"{model}-{scenario}-meta.json"
            meta_file.write_text(
                json.dumps(meta, indent=2, ensure_ascii=False), encoding="utf-8"
            )
            print(f"  ✓ {protocol}/streaming → {out_file.name}")
            return out_file

        # 非流式
        repeat = spec.get("repeat", 1)
        results: list[dict] = []

        for i in range(repeat):
            try:
                result = client.chat(messages, req_model, **http_kwargs)
                results.append(result)
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

        out_dir = output_base / protocol
        out_dir.mkdir(parents=True, exist_ok=True)

        output: dict[str, Any] = {
            "protocol": protocol,
            "streaming": False,
            "scenario": scenario,
            "model": req_model,
            "expect": spec["expect"],
            "request": {"model": req_model, "messages": messages},
            "extra_body_sent": spec.get("extra_body", {}),
            "response": results if len(results) != 1 else results[0],
        }

        out_file = out_dir / f"{model}-{scenario}.json"
        out_file.write_text(
            json.dumps(output, indent=2, ensure_ascii=False), encoding="utf-8"
        )
        print(f"  ✓ {protocol}/{scenario} → {out_file.name}")
        return out_file

    finally:
        if headers_backup is not None:
            client.headers = headers_backup


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
