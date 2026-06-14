"""
Provider 配置 — 仅 Phase 0 文档有记录的内容

每个 provider 的 base URL、认证 header 模板、模型列表。
加新 provider 在此文件末尾添加 dict 即可，capture_fixtures.py 无需改动。
"""



from typing import Any


PROVIDER_CONFIG: dict[str, dict[str, Any]] = {
    "minimax": {
        # ---- OpenAI 协议 ----
        "openai_url": "https://api.minimaxi.com/v1/chat/completions",
        "models_url": "https://api.minimaxi.com/v1/models",
        "usage_url": None,  # MiniMax 无公开用量查询 API
        "openai_headers": {
            "Authorization": None,  # 运行时填入 Bearer token
            "Content-Type": "application/json",
        },
        # ---- Anthropic 协议 ----
        "anthropic_url": "https://api.minimaxi.com/anthropic/v1/messages",
        "anthropic_models_url": "https://api.minimaxi.com/anthropic/v1/models",
        "anthropic_headers": {
            "x-api-key": None,
            "anthropic-version": "2023-06-01",
            "Content-Type": "application/json",
        },
        # 模型列表（Phase 0 文档有记录的）
        "openai_models": [
            "MiniMax-M2.7",
            "MiniMax-M2.7-highspeed",
            "MiniMax-M2.5",
            "MiniMax-M2.5-highspeed",
            "MiniMax-M2.1",
            "MiniMax-M2.1-highspeed",
            "MiniMax-M2",
        ],
        "anthropic_models": [
            "MiniMax-M2.7",
            "MiniMax-M2.7-highspeed",
            "MiniMax-M2.5",
            "MiniMax-M2.5-highspeed",
            "MiniMax-M2.1",
            "MiniMax-M2.1-highspeed",
            "MiniMax-M2",
        ],
        # MiniMax 特殊：业务错误走 HTTP 200 + base_resp.status_code
        "_error_via_base_resp": True,
    },
    "glm": {
        # ---- OpenAI 协议（Coding Plan 端点）----
        "openai_url": "https://open.bigmodel.cn/api/coding/paas/v4/chat/completions",
        "models_url": "https://open.bigmodel.cn/api/coding/paas/v4/models",
        "usage_url": "https://open.bigmodel.cn/api/monitor/usage/quota/limit",
        "openai_headers": {
            "Authorization": None,
            "Content-Type": "application/json",
        },
        # ---- Anthropic 协议 ----
        # 确认路径：Claude API 兼容页明确给出 cURL 示例
        "anthropic_url": "https://open.bigmodel.cn/api/anthropic/v1/messages",
        "anthropic_headers": {
            "x-api-key": None,
            "anthropic-version": "2023-06-01",
            "Content-Type": "application/json",
        },
        "openai_models": [
            "glm-5.1",
            "glm-5",
            "glm-5-turbo",
            "glm-4.7",
            "glm-4.7-flashx",
            "glm-4.7-flash",
            "glm-4.6",
            "glm-4.5-air",
            "glm-4.5-airx",
        ],
        "anthropic_models": [
            "glm-5.1",
            "glm-5",
            "glm-5-turbo",
            "glm-4.7",
            "glm-4.6",
        ],
        "_error_via_base_resp": False,
    },
    "deepseek": {
        # ---- OpenAI 协议 ----
        "openai_url": "https://api.deepseek.com/chat/completions",
        "models_url": "https://api.deepseek.com/models",
        "usage_url": "https://api.deepseek.com/user/balance",
        "openai_headers": {
            "Authorization": None,
            "Content-Type": "application/json",
        },
        # ---- Anthropic 协议 ----
        "anthropic_url": "https://api.deepseek.com/anthropic/v1/messages",
        "anthropic_headers": {
            "x-api-key": None,
            "anthropic-version": "2023-06-01",
            "Content-Type": "application/json",
        },
        "openai_models": [
            "deepseek-v4-flash",
            "deepseek-v4-pro",
        ],
        "anthropic_models": [
            "deepseek-v4-flash",
            "deepseek-v4-pro",
        ],
        "_error_via_base_resp": False,
    },
}
