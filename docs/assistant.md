# Build with MIRA

[Documentation](README.md) · [Right sidebar](sidebar.md) · [Developer README](../README.md)

Build with MIRA is an assistant that changes your model from plain-language requests, such as "make the left leg 40 mm thicker" or "add a mortise-and-tenon between the apron and leg 2". MIRA reads a summary of your model, calls BenchUp's own modeling tools, and reports what it changed. Each request is one undo step.

## Connect

1. Open **View → Build with MIRA** (Ctrl+Shift+M, or Cmd+Shift+M on macOS), or expand the **Build with MIRA** section at the bottom of the right sidebar.
2. Choose **Connect**, follow **Get a key** to create an API key at endlesstokens.ai, paste it, and choose **Connect**. BenchUp checks the key by loading the model list. The picker then defaults to a tool-capable Qwen model; you can pick any model from the list or type a model name.
3. Under **Advanced**, **Server address** accepts any OpenAI-compatible server. It must use `https`, except for `localhost` and `127.0.0.1`.

Use the **⋯** menu for **Change endpoint…** and **Forget API key**.

## Ask for changes

Type a request and press **Enter** to send. **Shift+Enter** adds a new line. **Up** in an empty box brings back your last request. **Stop** (or **Esc**) ends a running request; changes already made are kept as one undo step labeled "MIRA (stopped)".

While MIRA works, a "MIRA is editing…" banner appears over the viewport and edits of your own are paused so they cannot collide with MIRA's. Orbit, pan, zoom, and selecting stay available.

### Refer to parts of your design

- Type **@** and pick an object, group, or component by name. It becomes a removable `@Name` pill above the box.
- Drag a row from the Outliner onto the prompt box.
- Right-click selected parts and choose **Ask MIRA about this**; the panel opens with those parts attached.
- Keep **Include selection** on to send the current selection with every request. It shows as a dimmed "Selection (N)" pill.

References are matched by id, not by name, so renaming a part after attaching it does not break the reference.

### Review and undo

Each answer that changed the model has a card: "Created N · Modified N · Erased N · Joints N", with one row per part. Click a row to select the part and zoom to it. **Undo turn** reverts everything in that request and is available while it is still the latest history step; earlier steps are in the **History** section as usual. New and modified parts are selected after a request.

## Latch and unlatch

The panel is a normal sidebar section that you can reorder, split, and stack like the others. Choose the window icon in its toolbar to **unlatch** it:

- **Desktop:** it opens in its own window, which you can drag to another monitor and resize. Its position and size are remembered. Closing that window, or choosing **Latch back**, returns it to the sidebar. If the remembered position is off every screen (for example a monitor was unplugged), it opens near the main window instead.
- **Browser:** it becomes a floating window inside the canvas that you can move and resize.

The sidebar section shows "Open in a separate window" with **Latch back** and **Bring to front** while it is unlatched. Typing in the detached window never triggers editor shortcuts.

The shortcut key (default M, with Ctrl/Cmd+Shift) can be changed in **Settings → Shortcuts**.

## Privacy and key storage

When you send a request, BenchUp sends a summary of your model (object names, sizes, positions, tags, and joints) and your text to the configured server. A one-line notice shows the destination host until you choose **Don't show again**. Nothing is sent before you connect and send a request.

The API key is stored only on your device: in a private file in the BenchUp data folder on desktop, and in the browser's local storage on the web. It is never written to project files or to the sidebar preferences. **Forget API key** deletes it. The chat transcript is kept in memory for the session only; **New chat** clears it.

## Troubleshooting

- **Invalid API key:** the server rejected the key. Check that you copied all of it, or create a new one.
- **"This model doesn't support tool calls":** pick another model. MIRA needs a model with tool calling.
- **Requests fail or time out:** each key allows two parallel sessions; close other apps using the same key, then try again.
- **"Cannot reach the assistant server":** check your connection and the server address under **Advanced**.
- **Read-only shared project:** MIRA cannot edit a shared project you only have view access to.
- **Wrong results:** attach the parts with **@** so MIRA works on the right objects, and ask for one change at a time. Use **Undo turn** to go back.
