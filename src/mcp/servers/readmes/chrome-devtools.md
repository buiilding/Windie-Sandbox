# Chrome DevTools

Chrome DevTools lets Windie inspect, debug, and automate Chrome through an
explicitly selected browser connection.

## What it provides

The provider exposes browser pages, debugging, and automation tools through MCP.

## Setup

Windie-managed mode creates a separate persistent browser profile. Log into
websites in that profile once when needed; Windie reuses that profile later.

Existing-Chrome mode uses Chrome 144+'s approval flow. Open
`chrome://inspect/#remote-debugging`, enable **Allow remote debugging for this
browser instance**, and approve Windie's MCP connection when Chrome asks.

## Safety

The normal Chrome profile and its open tabs are not used in managed mode. In
existing-Chrome mode, the selected running Chrome is intentionally exposed to
MCP after explicit approval. Browser tools can access websites and perform
actions, so use approval mode when reviewing automation.
