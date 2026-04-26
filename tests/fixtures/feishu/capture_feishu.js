#!/usr/bin/env node
/**
 * Feishu API fixture capture script.
 *
 * Uses Lark SDK's WSClient (WebSocket mode) to receive Feishu events.
 * No ngrok, no public URL, no verification token needed.
 *
 * Usage:
 *   node capture_feishu.js <app_id> <app_secret>
 *
 * Example:
 *   node capture_feishu.js cli_a964e565a9f8dcb3 PD2GqeCIgxWS1jVhbC1pHbgPwRNQFkVU
 *
 * Output: Raw JSON files named by event_type and timestamp in this directory.
 */

const Lark = require('@larksuiteoapi/node-sdk');
const fs = require('fs');
const path = require('path');

const OUTPUT_DIR = __dirname;

const APP_ID = process.argv[2];
const APP_SECRET = process.argv[3];

if (!APP_ID || !APP_SECRET) {
  console.error(`Usage: node ${path.basename(__filename)} <app_id> <app_secret>`);
  console.error(`Example: node ${path.basename(__filename)} cli_a964e565a9f8dcb3 PD2Gxxx`);
  process.exit(1);
}

// Event types we care about for fixtures
const EVENT_TYPES = [
  'im.message.receive_v1',
  'im.message.message_read_v1',
  'im.message.reaction.created_v1',
  'im.message.reaction.deleted_v1',
  'im.chat.member.bot.added_v1',
  'im.chat.member.bot.deleted_v1',
  'card.action.trigger',
  'drive.notice.comment_add_v1',
];

const client = new Lark.Client({
  appId: APP_ID,
  appSecret: APP_SECRET,
  appType: Lark.AppType.SelfBuild,
  domain: Lark.Domain.Feishu,
});

const wsClient = new Lark.WSClient({
  appId: APP_ID,
  appSecret: APP_SECRET,
  domain: Lark.Domain.Feishu,
  loggerLevel: Lark.LoggerLevel.info,
});

// Build event handlers
const handlers = {};
for (const eventType of EVENT_TYPES) {
  handlers[eventType] = async (data) => {
    const eventId = data.header?.event_id ?? `no-event-id`;
    const ts = new Date().toISOString().replace(/[:.]/g, '-');
    const safeType = eventType.replace(/\./g, '-');
    const filename = `${safeType}-${eventId}-${ts}.json`;
    const filepath = path.join(OUTPUT_DIR, filename);

    const payload = JSON.stringify(data, null, 2);
    fs.writeFileSync(filepath, payload, 'utf8');
    console.log(`[SAVE] ${filename}`);
    console.log(`  event_type: ${eventType}`);
    console.log(`  message_id: ${data.message?.message_id ?? 'N/A'}`);
    console.log(`  chat_id: ${data.message?.chat_id ?? 'N/A'}`);
    console.log(`  sender: ${data.message?.sender?.id ?? 'N/A'}`);
  };
}

const dispatcher = new Lark.EventDispatcher({
  encryptKey: '',
  verificationToken: '',
});
dispatcher.register(handlers);

console.log('='.repeat(60));
console.log('Feishu Fixture Capture Script (WebSocket Mode)');
console.log('='.repeat(60));
console.log(`App ID:  ${APP_ID}`);
console.log(`Output:   ${OUTPUT_DIR}`);
console.log();
console.log('Listening for events...');
console.log('Press Ctrl+C to stop.');
console.log();
console.log('Event types:');
for (const t of EVENT_TYPES) {
  console.log(`  - ${t}`);
}
console.log();
console.log('No ngrok needed! Lark SDK connects outbound via WebSocket.');
console.log('='.repeat(60));
console.log();

wsClient.start({ eventDispatcher: dispatcher }).catch((err) => {
  console.error('[ERROR] WSClient failed:', err);
  process.exit(1);
});

// Handle graceful shutdown
process.on('SIGINT', () => {
  console.log('\nStopping...');
  wsClient.close?.({ force: true });
  process.exit(0);
});
