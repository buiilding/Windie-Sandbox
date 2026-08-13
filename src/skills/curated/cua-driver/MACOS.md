# CUA Driver on macOS

The CUA Driver may require macOS Accessibility and Screen Recording permission
for the applications it controls. If an action fails, inspect the application
state and report the missing permission instead of retrying blindly.

Prefer targeting an application by its stable bundle identifier when one is
available. Keep computer-control actions narrow and verify the visible result
after each action.
