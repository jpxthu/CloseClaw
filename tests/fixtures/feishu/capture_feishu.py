#!/usr/bin/env python3
"""
Feishu API fixture capture script.
Receives webhook events from Feishu bot and saves raw payloads as JSON files.

Usage:
    1. Run: python3 capture_feishu.py <port> <verification_token>
    2. Expose port via ngrok: ngrok http <port>
    3. Set the ngrok URL in Feishu Open Platform -> Event Subscription
    4. Send messages to the bot in Feishu to capture payloads

Output: Raw JSON files named by event_type and timestamp in this directory.
"""

import json
import os
import sys
import hashlib
import hmac
import time
from datetime import datetime
from http.server import HTTPServer, BaseHTTPRequestHandler

OUTPUT_DIR = os.path.dirname(os.path.abspath(__file__))
TOKEN = None

# Supported event types we care about for fixtures
EVENT_TYPES = {
    "im.message.receive_v1": "im-message",
    "im.message.message_read_v1": "im-message-read",
}


class FeishuHandler(BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        print(f"[{datetime.now().isoformat()}] {format % args}")

    def do_POST(self):
        if TOKEN is None:
            self.send_error_response(500, "No verification token set")
            return

        # Read raw body
        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length)

        # Validate signature
        # Feishu signs with: HMAC-SHA256 of verification_token + body
        sig = self.headers.get("X-Lark-Signature", "")
        expected = hmac.new(
            TOKEN.encode(), body, hashlib.sha256
        ).hexdigest()
        if not hmac.compare_digest(sig, expected):
            print(f"[WARN] Signature mismatch. Got {sig}")
            # Continue anyway for now — some events may not have signature
            # self.send_error_response(403, "Invalid signature")
            # return

        try:
            payload = json.loads(body)
        except json.JSONDecodeError as e:
            self.send_error_response(400, f"Invalid JSON: {e}")
            return

        event_type = payload.get("header", {}).get("event_type", "unknown")
        event_id = payload.get("header", {}).get("event_id", "no-event-id")

        # Save raw payload
        prefix = EVENT_TYPES.get(event_type, event_type.replace(".", "-"))
        ts = datetime.now().strftime("%Y%m%d-%H%M%S-%f")
        filename = f"{prefix}-{event_id}-{ts}.json"
        filepath = os.path.join(OUTPUT_DIR, filename)

        with open(filepath, "w", encoding="utf-8") as f:
            f.write(json.dumps(payload, ensure_ascii=False, indent=2))

        print(f"[SAVE] {filepath}  (event={event_type})")

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"code":0,"msg":"ok"}')

    def send_error_response(self, code, msg):
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps({"code": code, "msg": msg}).encode())


def main():
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <port> <verification_token>")
        print(f"Example: {sys.argv[0]} 8080 your-verification-token")
        sys.exit(1)

    port = int(sys.argv[1])
    global TOKEN
    TOKEN = sys.argv[2]

    os.makedirs(OUTPUT_DIR, exist_ok=True)
    print(f"Listening on http://0.0.0.0:{port}")
    print(f"Output dir: {OUTPUT_DIR}")
    print(f"Verification token set: {TOKEN[:4]}...")
    print()
    print("Next steps:")
    print(f"  1. ngrok http {port}")
    print(f"  2. Copy the Forwarding URL (https://xxx.ngrok.io)")
    print(f"  3. Set it in Feishu Open Platform -> Event Subscription URL")
    print(f"  4. Send messages to your bot in Feishu")
    print()

    server = HTTPServer(("0.0.0.0", port), FeishuHandler)
    print(f"Server started. Press Ctrl+C to stop.")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nStopped.")
        server.shutdown()


if __name__ == "__main__":
    main()
