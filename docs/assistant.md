# Build with MIRA

[Documentation](README.md) · [Right sidebar](sidebar.md) · [Developer README](../README.md)

Build with MIRA is an assistant that changes your model from plain-language requests, such as "make the left leg 40 mm thicker" or "add a mortise-and-tenon between the apron and leg 2". MIRA reads a summary of your model, calls BenchUp's own modeling tools, and reports what it changed. Each request is one undo step.

## Connect

1. Open **View → Build with MIRA** (Ctrl+Shift+M, or Cmd+Shift+M on macOS), or expand the **Build with MIRA** section at the bottom of the right sidebar.
2. Choose **Connect**, follow **Get a key** to create an API key at endlesstokens.ai, paste it, and choose **Connect**. BenchUp checks the key by loading the model list. The picker then defaults to a tool-capable Qwen model; you can pick any model from the list or type a model name. A model you picked or typed is kept even if the server's list does not include it.
3. Under **Advanced**, **Server address** accepts any OpenAI-compatible server. It must use `https`, except for `localhost` and `127.0.0.1`. A new address is remembered only after the server has accepted the key; if the attempt fails, BenchUp stays connected to the previous server.

Use the **⋯** menu for **Change endpoint…** and **Forget API key**.

## Ask for changes

Type a request and press **Enter** to send. **Shift+Enter** adds a new line. **Up** in an empty box brings back your last request. **Stop** (or **Esc**) ends a running request; changes already made are kept as one undo step labeled "MIRA (stopped)".

While MIRA works, a "MIRA is editing…" banner appears over the viewport and edits of your own are paused so they cannot collide with MIRA's. Orbit, pan, zoom, and selecting stay available.

### Refer to parts of your design

- Type **@** and pick an object, group, or component by name. It becomes a removable `@Name` pill above the box.
- Drag a row from the Outliner onto the prompt box.
- Right-click selected parts and choose **Ask MIRA about this**; the panel opens with those parts attached.
- Keep **Include selection** on to send the current selection with every request. It shows as a dimmed "Selection (N)" pill. Turn it off and your selection is not sent at all: it is left out of the model summary and MIRA is told that you chose not to share it.

References are matched by id, not by name, so renaming a part after attaching it does not break the reference.

### Review and undo

Each answer that changed the model has a card: "Created N · Modified N · Erased N · Joints N", with one row per part. Click a row to select the part and zoom to it. **Undo turn** reverts everything in that request and is available while it is still the latest history step; earlier steps are in the **History** section as usual. New and modified parts are selected after a request. If you were editing inside a group or component, you are back in it when the request ends, and **Undo turn** leaves you there too. If more than 12 parts changed, the card ends with "+N more" (created, modified, joints and erased parts all count).

## Latch and unlatch

The panel is a normal sidebar section that you can reorder, split, and stack like the others. Choose the window icon in its toolbar to **unlatch** it:

- **Desktop:** it opens in its own window, which you can drag to another monitor and resize. Its position and size are remembered. Closing that window, or choosing **Latch back**, returns it to the sidebar. If the remembered position is off every screen (for example a monitor was unplugged), it opens near the main window instead.
- **Browser:** it becomes a floating window inside the canvas that you can move and resize.

The sidebar section shows "Open in a separate window" with **Latch back** and **Bring to front** while it is unlatched; Bring to front raises the window above everything else, in the browser too, and focuses the prompt. Typing in the detached window never triggers editor shortcuts.

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
- **"... rolled back because you no longer have edit access":** in a shared project your editing access lapsed before MIRA's changes could be saved, so BenchUp undid them as a whole and the card is not shown. Reconnect and ask again.
- **"not-leased" or "The shared project changed; MIRA stopped":** in a shared project MIRA may only edit what you have selected (your edit lease), and a turn ends if the server replaces the project or your editing access lapses (access is checked before each edit MIRA makes). MIRA can only select parts you hold the lease on. Select the parts and ask again.
- **"result out of range" / "too many ..." errors:** MIRA checks every request before applying it (at most 32 tool calls per reply, 1000 polyline points, 500 ids; positions within 100 m, results within 1 km). In the browser build a bug in a tool cannot be caught and recovered (WebAssembly aborts on a panic and the page must be reloaded), so these checks are the only guard there; report any crash with the prompt that caused it.
- **Wrong results:** attach the parts with **@** so MIRA works on the right objects, and ask for one change at a time. Use **Undo turn** to go back.

## For contributors

How the assistant is designed and how to extend it (architecture, every tool, the LLM protocol, security model, tests and recipes): see [mira/README.md](mira/README.md).
