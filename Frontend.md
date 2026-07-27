## Browser UI., dev/windie-ui/
Mental Model
- index.html: the stage, pure structure: empty containers (#status, #messages, #input, #send), the inline CSS, and one line that loads app.js. No logic, no state, just declares what exists on the screen. PURE STRUCTURE.
- api.js: acts but never listens, turn a function call into a HTTP request to Windie api request. GET, POST
- stream.js: listen to windie's session's event stream and hands backend each event to apps.js as it arrives, uses fetch not eventsource.
- app.js: orchestrator, owns the state (conversationid, headmessageid), grabs the elements in index,js, calls api.js in message->session order, hands the resulting session id to stream.js, listen to each event stream.js gives it, stream the result to screen.
- format.js: formatter for response text, only format user bubbles, assistant text, streaming previews, system prompts.