Mental model:
- conversation.rs: what a message is.
- context.rs: which messages go to the model
- llm.rs: talks HTTP to Bifrost
- output.rs: print streamed text
- store.rs: persist messages
- perf.rs: runtime performance tests

Conversations are much like working with trees:
- append: add child message
- rm: remove node, reconnect or delete branch depending behavior
- truncate: remove descendants after a node on current path
- fork: copy path into new conversation, or later create branch
- update: mutate node content
- query: append assistant child to current leaf
- show: show one path of the tree.