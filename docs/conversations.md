# Saved conversations

Buntini saves a conversation as a thread containing all its turns. History shows one entry per thread with its title and turn count. Opening it restores that thread; the next message continues it. **New topic** creates a separate empty thread. Earlier threads stay available in History.

Conversation turns load 100 at a time; **Load earlier** retrieves the rest. History lists 30 threads at a time with **Load older conversations**. Timestamps are paired with stable IDs for pagination, so messages created at the same time are not skipped. A model still receives only the recent context that fits its history budget and sharing permissions, rather than every stored turn.

Task and approval IDs do not change when threads are opened or switched. A resumed task keeps its original conversation, tool journal, approval inputs and receipts. Switching is blocked during execution. The UI opens a saved task's original conversation before continuing or reviewing its next step.

Automatic recurring search results go into one **Saved search: …** conversation per saved search, without switching the selected thread or entering another thread's model history. **Check now** is a user-initiated turn in the selected conversation.

## Existing installations

Earlier versions stored a flat stream and only the latest New topic timestamp. The migration cannot recover earlier thread boundaries. It groups turns up to that timestamp into **Earlier conversations (legacy)**, and turns after it into **Current conversation (imported)**. If no timestamp exists, all existing turns form one legacy conversation. This conservative migration preserves every turn and does not invent old conversations. New threads have their own stable IDs from then on.

The SQLite conversation mapping is separate from the task JSON and workflow journal. Every task insertion, including a direct workflow checkpoint, assigns membership once. Replacing a task during approval or interrupted-run recovery preserves the existing mapping. Export includes all turns, thread metadata, the active thread ID and task membership. Clear conversation history removes these along with workflow history; Delete everything removes them too.

## Verification

`cargo test --manifest-path src-tauri/Cargo.toml chat_threads` covers legacy migration/restart, missing boundaries, 235 timestamp-tied turns across pages, cross-thread cursor rejection, 105 saved threads, export/clear/delete, and checkpoint/recovery/resume while a different thread is active. `node --test src/thread.test.ts` checks switching without mixing turns, preserving loaded pages on refresh, replacing task results and deterministic timestamp ties. All fixtures use disposable databases and synthetic task records.
