Module: nod-ide

// Sprint 41g — Save, Save As, Recent files submenu on top of Sprint 41e's
// File / Help menu bar.
//
// Sprint 41e shipped File → Open / Exit + Help → About. Sprint 41g extends
// the File menu so it now looks like:
//
//   File
//     Open...      (cmd-id 100)
//     Save         (cmd-id 101)
//     Save As...   (cmd-id 102)
//     ────────
//     Recent ▶                                  (NEW submenu)
//       1. F:\scratch\foo.dylan   (cmd-id 301)
//       2. F:\scratch\bar.txt     (cmd-id 302)
//       (etc., max 5 entries)
//     ────────
//     Exit         (cmd-id 199)
//   Help
//     About        (cmd-id 200)
//
// The window title shows the current file's basename, e.g.
// "foo.dylan - NewOpenDylan IDE".
//
// The recent-files list persists across runs in
// F:\scratch\nod-ide-recent.txt — one absolute path per line,
// most-recent first, capped at 5. Sprint 42a: persistence + dedup + cap
// is now pure Dylan (`nod-load-recent` / `nod-add-recent` /
// `nod-save-recent`) built on the byte-string ops landed in stdlib —
// the old `nod_load_recent` / `nod_add_recent` Rust shims are gone.
//
// IMPORTANT — the editor is still read-only (no cursor, no editing —
// that's Sprint 41h or later). Save in this sprint rewrites the file
// with its current in-memory contents. That's intentional: the plumbing
// (file picker, byte-string write, recent-list maintenance, title bar)
// is ready for when editing arrives.
//
// MessageBoxW-from-WNDPROC remains broken (Sprint 41f investigation,
// see docs/duim-research/07-probe-findings.md); Help → About still
// uses the SetWindowTextW workaround.

define c-function CreateWindowExW
  (dwExStyle :: <c-int>, lpClassName :: <c-pointer>, lpWindowName :: <c-wide-string>,
   dwStyle :: <c-int>, x :: <c-int>, y :: <c-int>, nWidth :: <c-int>, nHeight :: <c-int>,
   hWndParent :: <c-pointer>, hMenu :: <c-pointer>, hInstance :: <c-pointer>,
   lpParam :: <c-pointer>)
 => (hwnd :: <c-pointer>);
    library: "user32.dll";
end;

define c-function ShowWindow
  (hwnd :: <c-pointer>, nCmdShow :: <c-int>)
 => (was-visible :: <c-bool>);
    library: "user32.dll";
end;

define c-function UpdateWindow
  (hwnd :: <c-pointer>)
 => (success :: <c-bool>);
    library: "user32.dll";
end;

define c-function InvalidateRect
  (hwnd :: <c-pointer>, lpRect :: <c-pointer>, bErase :: <c-bool>)
 => (success :: <c-bool>);
    library: "user32.dll";
end;

define c-function DefWindowProcW
  (hwnd :: <c-pointer>, msg :: <c-int>,
   wparam :: <c-pointer>, lparam :: <c-pointer>)
 => (lresult :: <c-pointer>);
    library: "user32.dll";
end;

define c-function PostQuitMessage
  (exit-code :: <c-int>)
 => ();
    library: "user32.dll";
end;

// Sprint 41e — menu API declarations (explicit so the AppendMenuW
// 4th-arg lpNewItem stays `<c-wide-string>` for menu items; we pass
// the HMENU for popup submenus via the 3rd-arg `uIDNewItem` which is
// typed `<c-pointer>` to accept both fixnum ids and HMENU values).
define c-function CreateMenu
  ()
 => (hmenu :: <c-pointer>);
    library: "user32.dll";
end;

define c-function CreatePopupMenu
  ()
 => (hmenu :: <c-pointer>);
    library: "user32.dll";
end;

define c-function AppendMenuW
  (hmenu :: <c-pointer>, uFlags :: <c-int>, uIDNewItem :: <c-pointer>,
   lpNewItem :: <c-wide-string>)
 => (success :: <c-bool>);
    library: "user32.dll";
end;

// Sprint 41g — menu rebuild helpers. `RemoveMenu` with MF_BYPOSITION
// (1024) removes the item at the given index; positions shift after
// removal so calling with position 0 repeatedly tears the submenu
// down. `DrawMenuBar` forces the OS to repaint the menu bar after
// programmatic changes (the submenu's own popup is rebuilt on the
// next click so we don't have to invalidate it explicitly).
define c-function RemoveMenu
  (hmenu :: <c-pointer>, uPosition :: <c-int>, uFlags :: <c-int>)
 => (success :: <c-bool>);
    library: "user32.dll";
end;

define c-function DrawMenuBar
  (hwnd :: <c-pointer>)
 => (success :: <c-bool>);
    library: "user32.dll";
end;

// SetWindowTextW is the Help → About workaround (see Sprint 41e
// notes) and is also what we use for the per-file title.
define c-function SetWindowTextW
  (hwnd :: <c-pointer>, lpString :: <c-wide-string>)
 => (success :: <c-bool>);
    library: "user32.dll";
end;

define c-function MessageBoxW
  (hwnd :: <c-pointer>, lpText :: <c-wide-string>, lpCaption :: <c-wide-string>,
   uType :: <c-int>)
 => (result :: <c-int>);
    library: "user32.dll";
end;

// ─── Sprint 42a — pure-Dylan helpers (replace retired Rust shims) ─────────
//
// All the IDE's text-buffer scanning and recent-files persistence is now
// pure Dylan over the Sprint 42a `<byte-string>` ops: `size`, `element`
// (the byte at an index), `concatenate`, `copy-sequence`, plus the
// Sprint 42a `=` method on `<byte-string>` (content equality). The five
// Rust shims (`nod_count_newlines`, `nod_max_line_chars`, `nod_basename`,
// `nod_load_recent`, `nod_add_recent`) are gone — this is the proof
// that the byte-string stdlib methods are usable for real work.
//
// Where we still call Rust:
//   * `%read-file` / `%write-file` — file I/O proper (Sprint 41b / 41g).
//   * `%byte-string-element` — primitive byte read (5-op surface).
//   * `pair` / `head` / `tail` / `empty?` / `nil` — list builtins
//     (Sprint 16; can't be specialised on `<byte-string>` yet — see the
//     `empty?` note in stdlib.dylan).
// Everything else here is plain Dylan calling the stdlib methods.

// ── Count newlines in a byte-string buffer ────────────────────────────────
// Returns 1 + the number of `\n` bytes (the line count, matching the
// "lines = newlines + 1" convention the IDE used pre-42a).

define function nod-count-newlines (s) => (lines)
  let n = size(s);
  let count = 1;
  let i = 0;
  until (i = n)
    if (element(s, i) = 10)        // 10 = '\n'
      count := count + 1;
    else
      #f
    end;
    i := i + 1;
  end;
  count
end function;

// ── Longest line length in bytes ──────────────────────────────────────────

define function nod-max-line-chars (s) => (best)
  let n = size(s);
  let best = 0;
  let cur = 0;
  let i = 0;
  until (i = n)
    if (element(s, i) = 10)        // 10 = '\n'
      if (cur > best) best := cur; else #f end;
      cur := 0;
    else
      cur := cur + 1;
    end;
    i := i + 1;
  end;
  if (cur > best) cur else best end
end function;

// ── basename(path) — last `\`-or-`/`-separated component ──────────────────
// Returns the empty string for nil or an empty byte-string.

define function nod-basename (path) => (base)
  if (empty?(path))               // empty?(nil) = #t for the list-builtin
    ""
  else
    let n = size(path);
    if (n = 0)
      path
    else
      // Scan for the LAST '\' (92) or '/' (47). sep-pos = -1 means
      // "no separator found, return path unchanged".
      let sep-pos = -1;
      let i = 0;
      until (i = n)
        let b = element(path, i);
        if (b = 92 | b = 47)
          sep-pos := i;
        else
          #f
        end;
        i := i + 1;
      end;
      if (sep-pos < 0)
        path
      else
        copy-sequence(path, sep-pos + 1, n)
      end
    end
  end
end function;

// ── List helpers — reverse, filter-out, take-first ────────────────────────
//
// Recent-files manipulation is built on a small kit of list ops. The
// stdlib's `do` / `map` / `reduce` exist but want a first-class function
// arg; for the simple loops we have, raw `head`/`tail`/`pair` walks are
// clearer.

define function nod-reverse-list (lst) => (rev)
  let result = nil();
  let cursor = lst;
  until (empty?(cursor))
    result := pair(head(cursor), result);
    cursor := tail(cursor);
  end;
  result
end function;

define function nod-remove-from-list (item, lst) => (filtered)
  // Walk lst, dropping any entry that equals `item` (byte-string `=`).
  // Build reversed, then flip — keeps the original order without an
  // append.
  let acc = nil();
  let cursor = lst;
  until (empty?(cursor))
    let p = head(cursor);
    if (p = item)
      #f   // skip — drop this entry
    else
      acc := pair(p, acc);
    end;
    cursor := tail(cursor);
  end;
  nod-reverse-list(acc)
end function;

define function nod-take-first (lst, n) => (taken)
  let acc = nil();
  let cursor = lst;
  let i = 0;
  until (empty?(cursor) | i = n)
    acc := pair(head(cursor), acc);
    cursor := tail(cursor);
    i := i + 1;
  end;
  nod-reverse-list(acc)
end function;

// ── Split a byte-string on `\n`, returning a list of byte-strings ─────────

define function nod-split-on-newline (bytes) => (lines)
  let n = size(bytes);
  if (n = 0)
    nil()
  else
    let acc = nil();
    let lo = 0;
    let i = 0;
    until (i = n)
      if (element(bytes, i) = 10)
        let line = copy-sequence(bytes, lo, i);
        acc := pair(line, acc);
        lo := i + 1;
      else
        #f
      end;
      i := i + 1;
    end;
    // Trailing segment (after the last '\n', or the only segment if
    // there's no '\n' at all). Skip if the buffer ended with a newline.
    if (lo < n)
      let tail-line = copy-sequence(bytes, lo, n);
      acc := pair(tail-line, acc);
    else
      #f
    end;
    nod-reverse-list(acc)
  end
end function;

// ── Join a list of byte-strings with `\n` ─────────────────────────────────

define function nod-join-with-newline (lst) => (joined)
  if (empty?(lst))
    ""
  else
    let cursor = lst;
    let result = head(cursor);
    cursor := tail(cursor);
    until (empty?(cursor))
      result := concatenate(result, "\n");
      result := concatenate(result, head(cursor));
      cursor := tail(cursor);
    end;
    result
  end
end function;

// ── Recent-files load / save / prepend-dedupe ─────────────────────────────
// Persists to F:\scratch\nod-ide-recent.txt — one path per line,
// most-recent first, capped at 5 entries. Missing/empty file → empty
// list. Write errors are silently ignored (best-effort persistence;
// the IDE keeps working, the user just loses the entry on next launch).

define function nod-load-recent () => (lst)
  let bytes = %read-file("F:\\scratch\\nod-ide-recent.txt");
  nod-split-on-newline(bytes)
end function;

define function nod-save-recent (recent-list) => ()
  let serialized = nod-join-with-newline(recent-list);
  %write-file("F:\\scratch\\nod-ide-recent.txt", serialized);
  #f
end function;

define function nod-add-recent (path, recent-list) => (new-list)
  // nil or empty-string path → no-op (treat as "nothing to remember").
  if (empty?(path))
    recent-list
  elseif (size(path) = 0)
    recent-list
  else
    let deduped  = nod-remove-from-list(path, recent-list);
    let prepended = pair(path, deduped);
    let capped    = nod-take-first(prepended, 5);
    nod-save-recent(capped);
    capped
  end
end function;

// ─── Helper: walk a recent-paths list, rebuild a submenu ────────────────
//
// Tears down every item in `recent-menu` (RemoveMenu at position 0
// while it returns success) and re-appends one MF_STRING entry per
// path. If the list is empty, appends a single disabled "(empty)" item
// (MF_GRAYED = 1) so the submenu is still visible to the user.
//
// Command ids are 301..305 — five slots for the five recent entries.
// Walking the spine with `pair`/`head`/`tail`/`empty?` is the standard
// Sprint 16 list-iteration pattern; the loop terminates either when
// the list is exhausted or when `i` reaches the 5-entry cap (defensive
// — `nod_add_recent` already caps at 5).

define function rebuild-recent-submenu (recent-menu, paths) => ()
  // Tear down whatever was there. RemoveMenu returns #t (BOOL true)
  // on success / #f when the position is out of range — that's our
  // natural loop guard.
  let removed = RemoveMenu(recent-menu, 0, 1024);
  until (~ removed)
    removed := RemoveMenu(recent-menu, 0, 1024);
  end;
  if (empty?(paths))
    // (empty) placeholder — disabled (MF_GRAYED = 1), no cmd-id.
    AppendMenuW(recent-menu, 1, 0, "(empty)");
  else
    let cursor = paths;
    let i = 0;
    until (empty?(cursor) | i > 4)
      let p = head(cursor);
      let label = nod-basename(p);
      AppendMenuW(recent-menu, 0, 301 + i, label);
      cursor := tail(cursor);
      i := i + 1;
    end;
  end;
end function;

// ─── Helper: set the window title to "basename - NewOpenDylan IDE" ──────
//
// If `path` is nil / empty, sets the title to the bare program name.
// Otherwise: "<basename> — NewOpenDylan IDE". Sprint 42a finally lets
// us build the title via `concatenate` (was held back in 41g for lack
// of string-concat on `<byte-string>`).

define function update-title (hwnd, path) => ()
  if (empty?(path))
    SetWindowTextW(hwnd, "NewOpenDylan IDE");
  else
    let base   = nod-basename(path);
    let suffix = concatenate(base, " - NewOpenDylan IDE");
    SetWindowTextW(hwnd, suffix);
  end;
end function;

define function main () => ()
  let arg-path = %argv1();
  let source-text = if (empty?(arg-path))
                      "nod-ide: no argv[1] supplied; pass a Dylan source path as the first argument."
                    else
                      let bytes = %read-file(arg-path);
                      if (empty?(bytes))
                        "nod-ide: could not read the file passed via argv[1]."
                      else
                        bytes
                      end
                    end;
  // Sprint 41g — current-path is a captured cell (Sprint 24 auto cell
  // promotion: any `let`-bound name assigned inside the WNDPROC
  // closure becomes a cell). Same machinery that promoted source-text
  // in Sprint 41e.
  let current-path = arg-path;
  let recent-paths = nod-load-recent();
  let d3d-device   = %d3d11-create-device();
  let dxgi-factory = %dxgi-factory-from-d3d-device(d3d-device);
  let dxgi-device  = %dxgi-device-from-d3d-device(d3d-device);
  let d2d-factory  = %d2d-create-factory();
  let d2d-device   = %d2d-create-device(d2d-factory, dxgi-device);
  let dc           = %d2d-create-device-context(d2d-device);
  let dwrite       = %dwrite-create-factory();
  let format       = %dwrite-create-text-format(dwrite, "Consolas", 1400, "en-us");
  let buffer-lines    = nod-count-newlines(source-text);
  let buffer-max-cols = nod-max-line-chars(source-text);
  let char-width  = 8;
  let line-height = 18;
  let pad = 8;
  let client-width-px  = buffer-max-cols * char-width;
  let client-height-px = buffer-lines * line-height;
  let window-width    = 1024;
  let window-height   = 768;
  let viewport-width-px  = 1024;
  let viewport-height-px = 768;
  let scroll-x-px = 0;
  let scroll-y-px = 0;
  let swap   = 0;
  let bitmap = 0;
  // Sprint 41g — build the menu bar HERE (before the WNDPROC closure
  // captures `recent-menu`) so the WM_COMMAND handler can call
  // `rebuild-recent-submenu` on `recent-menu` when the recent list
  // changes.
  let menu-bar = CreateMenu();
  let file-menu = CreatePopupMenu();
  let recent-menu = CreatePopupMenu();
  // AppendMenuW flag values (Win32 MF_*):
  //   MF_STRING    = 0      — plain text item (default)
  //   MF_GRAYED    = 1      — disabled / greyed
  //   MF_POPUP     = 16     — uIDNewItem is a submenu HMENU
  //   MF_SEPARATOR = 2048   — horizontal divider (lpNewItem ignored)
  AppendMenuW(file-menu, 0,    100, "&Open...\tCtrl+O");
  AppendMenuW(file-menu, 0,    101, "&Save\tCtrl+S");
  AppendMenuW(file-menu, 0,    102, "Save &As...\tCtrl+Shift+S");
  AppendMenuW(file-menu, 2048, 0,   "");
  AppendMenuW(file-menu, 16,   recent-menu, "&Recent");
  AppendMenuW(file-menu, 2048, 0,   "");
  AppendMenuW(file-menu, 0,    199, "E&xit\tAlt+F4");
  AppendMenuW(menu-bar,  16,   file-menu, "&File");
  let help-menu = CreatePopupMenu();
  AppendMenuW(help-menu, 0,    200, "&About");
  AppendMenuW(menu-bar,  16,   help-menu, "&Help");
  rebuild-recent-submenu(recent-menu, recent-paths);
  let wp = method (hwnd, msg, wparam, lparam)
             if (msg = 15)  // WM_PAINT
               if (swap ~= 0)
                 if (bitmap = 0)
                   bitmap := %d2d-create-bitmap-from-swap-chain(dc, swap);
                 else 0 end;
                 %d2d-set-target(dc, bitmap);
                 %d2d-begin-draw(dc);
                 %d2d-clear(dc, 255, 255, 255, 255);
                 let brush  = %d2d-create-solid-color-brush(dc, 0, 0, 0, 255);
                 let layout = %dwrite-create-text-layout(dwrite, source-text, format,
                                                         client-width-px, client-height-px);
                 %d2d-draw-text-layout(dc, pad - scroll-x-px, pad - scroll-y-px, layout, brush);
                 %d2d-end-draw(dc);
                 %com-release(brush);
                 %com-release(layout);
                 %dxgi-swap-chain-present(swap);
               else 0 end;
               0
             elseif (msg = 5)  // WM_SIZE
               if (swap ~= 0 & wparam ~= 1)
                 let new-w = %lo-word(lparam);
                 let new-h = %hi-word(lparam);
                 if (new-w > 0 & new-h > 0)
                   if (bitmap ~= 0)
                     %d2d-set-target(dc, 0);
                     %com-release(bitmap);
                     bitmap := 0;
                   else 0 end;
                   window-width  := new-w;
                   window-height := new-h;
                   viewport-width-px  := new-w;
                   viewport-height-px := new-h;
                   %dxgi-swap-chain-resize-buffers(swap, new-w, new-h);
                   %set-scroll-info(hwnd, 1, 0, client-height-px, viewport-height-px, scroll-y-px, 1);
                   %set-scroll-info(hwnd, 0, 0, client-width-px,  viewport-width-px,  scroll-x-px, 1);
                 else 0 end;
               else 0 end;
               0
             elseif (msg = 277)  // WM_VSCROLL
               let action = %lo-word(wparam);
               let new-pos = if (action = 0)        // SB_LINEUP
                               scroll-y-px - line-height
                             elseif (action = 1)    // SB_LINEDOWN
                               scroll-y-px + line-height
                             elseif (action = 2)    // SB_PAGEUP
                               scroll-y-px - (viewport-height-px - line-height)
                             elseif (action = 3)    // SB_PAGEDOWN
                               scroll-y-px + (viewport-height-px - line-height)
                             elseif (action = 4)    // SB_THUMBPOSITION
                               %hi-word(wparam)
                             elseif (action = 5)    // SB_THUMBTRACK
                               %hi-word(wparam)
                             elseif (action = 6)    // SB_TOP (Home)
                               0
                             elseif (action = 7)    // SB_BOTTOM (End)
                               client-height-px - viewport-height-px
                             else
                               scroll-y-px           // SB_ENDSCROLL / unknown
                             end;
               let max-scroll = if (client-height-px > viewport-height-px)
                                  client-height-px - viewport-height-px
                                else 0 end;
               let clamped = if (new-pos < 0) 0
                             elseif (new-pos > max-scroll) max-scroll
                             else new-pos end;
               if (clamped ~= scroll-y-px)
                 scroll-y-px := clamped;
                 %set-scroll-info(hwnd, 1, 0, client-height-px, viewport-height-px, clamped, 1);
                 InvalidateRect(hwnd, 0, 0);
               else 0 end;
               0
             elseif (msg = 276)  // WM_HSCROLL
               let action = %lo-word(wparam);
               let new-pos = if (action = 0)        // SB_LINELEFT
                               scroll-x-px - char-width
                             elseif (action = 1)    // SB_LINERIGHT
                               scroll-x-px + char-width
                             elseif (action = 2)    // SB_PAGELEFT
                               scroll-x-px - (viewport-width-px - char-width)
                             elseif (action = 3)    // SB_PAGERIGHT
                               scroll-x-px + (viewport-width-px - char-width)
                             elseif (action = 4)    // SB_THUMBPOSITION
                               %hi-word(wparam)
                             elseif (action = 5)    // SB_THUMBTRACK
                               %hi-word(wparam)
                             elseif (action = 6)    // SB_LEFT
                               0
                             elseif (action = 7)    // SB_RIGHT
                               client-width-px - viewport-width-px
                             else
                               scroll-x-px
                             end;
               let max-scroll = if (client-width-px > viewport-width-px)
                                  client-width-px - viewport-width-px
                                else 0 end;
               let clamped = if (new-pos < 0) 0
                             elseif (new-pos > max-scroll) max-scroll
                             else new-pos end;
               if (clamped ~= scroll-x-px)
                 scroll-x-px := clamped;
                 %set-scroll-info(hwnd, 0, 0, client-width-px, viewport-width-px, clamped, 1);
                 InvalidateRect(hwnd, 0, 0);
               else 0 end;
               0
             elseif (msg = 522)  // WM_MOUSEWHEEL
               let raw-delta = %hi-word(wparam);
               let signed-delta = if (raw-delta > 32767)
                                    raw-delta - 65536
                                  else
                                    raw-delta
                                  end;
               let flags = %lo-word(wparam);
               let shift-bit = (flags / 4) - (flags / 8) * 2;
               if (shift-bit = 1)
                 let chars-to-scroll = -1 * signed-delta * 3 / 120;
                 let new-pos = scroll-x-px + chars-to-scroll * char-width;
                 let max-scroll = if (client-width-px > viewport-width-px)
                                    client-width-px - viewport-width-px
                                  else 0 end;
                 let clamped = if (new-pos < 0) 0
                               elseif (new-pos > max-scroll) max-scroll
                               else new-pos end;
                 if (clamped ~= scroll-x-px)
                   scroll-x-px := clamped;
                   %set-scroll-info(hwnd, 0, 0, client-width-px, viewport-width-px, clamped, 1);
                   InvalidateRect(hwnd, 0, 0);
                 else 0 end;
               else
                 let lines-to-scroll = -1 * signed-delta * 3 / 120;
                 let new-pos = scroll-y-px + lines-to-scroll * line-height;
                 let max-scroll = if (client-height-px > viewport-height-px)
                                    client-height-px - viewport-height-px
                                  else 0 end;
                 let clamped = if (new-pos < 0) 0
                               elseif (new-pos > max-scroll) max-scroll
                               else new-pos end;
                 if (clamped ~= scroll-y-px)
                   scroll-y-px := clamped;
                   %set-scroll-info(hwnd, 1, 0, client-height-px, viewport-height-px, clamped, 1);
                   InvalidateRect(hwnd, 0, 0);
                 else 0 end;
               end;
               0
             elseif (msg = 256)  // WM_KEYDOWN
               let vk = %lo-word(wparam);
               let v-max = if (client-height-px > viewport-height-px)
                             client-height-px - viewport-height-px
                           else 0 end;
               let h-max = if (client-width-px > viewport-width-px)
                             client-width-px - viewport-width-px
                           else 0 end;
               if (vk = 33)        // VK_PRIOR (PgUp)
                 let new-pos = scroll-y-px - (viewport-height-px - line-height);
                 let clamped = if (new-pos < 0) 0
                               elseif (new-pos > v-max) v-max
                               else new-pos end;
                 if (clamped ~= scroll-y-px)
                   scroll-y-px := clamped;
                   %set-scroll-info(hwnd, 1, 0, client-height-px, viewport-height-px, clamped, 1);
                   InvalidateRect(hwnd, 0, 0);
                 else 0 end;
               elseif (vk = 34)    // VK_NEXT (PgDn)
                 let new-pos = scroll-y-px + (viewport-height-px - line-height);
                 let clamped = if (new-pos < 0) 0
                               elseif (new-pos > v-max) v-max
                               else new-pos end;
                 if (clamped ~= scroll-y-px)
                   scroll-y-px := clamped;
                   %set-scroll-info(hwnd, 1, 0, client-height-px, viewport-height-px, clamped, 1);
                   InvalidateRect(hwnd, 0, 0);
                 else 0 end;
               elseif (vk = 36)    // VK_HOME — top-left corner
                 if (scroll-y-px ~= 0 | scroll-x-px ~= 0)
                   scroll-y-px := 0;
                   scroll-x-px := 0;
                   %set-scroll-info(hwnd, 1, 0, client-height-px, viewport-height-px, 0, 1);
                   %set-scroll-info(hwnd, 0, 0, client-width-px,  viewport-width-px,  0, 1);
                   InvalidateRect(hwnd, 0, 0);
                 else 0 end;
               elseif (vk = 35)    // VK_END — bottom of file
                 if (scroll-y-px ~= v-max)
                   scroll-y-px := v-max;
                   %set-scroll-info(hwnd, 1, 0, client-height-px, viewport-height-px, v-max, 1);
                   InvalidateRect(hwnd, 0, 0);
                 else 0 end;
               elseif (vk = 37)    // VK_LEFT
                 let new-pos = scroll-x-px - char-width;
                 let clamped = if (new-pos < 0) 0
                               elseif (new-pos > h-max) h-max
                               else new-pos end;
                 if (clamped ~= scroll-x-px)
                   scroll-x-px := clamped;
                   %set-scroll-info(hwnd, 0, 0, client-width-px, viewport-width-px, clamped, 1);
                   InvalidateRect(hwnd, 0, 0);
                 else 0 end;
               elseif (vk = 39)    // VK_RIGHT
                 let new-pos = scroll-x-px + char-width;
                 let clamped = if (new-pos < 0) 0
                               elseif (new-pos > h-max) h-max
                               else new-pos end;
                 if (clamped ~= scroll-x-px)
                   scroll-x-px := clamped;
                   %set-scroll-info(hwnd, 0, 0, client-width-px, viewport-width-px, clamped, 1);
                   InvalidateRect(hwnd, 0, 0);
                 else 0 end;
               else 0 end;
               0
             elseif (msg = 273)  // WM_COMMAND — Sprint 41e/g menu dispatch
               // Menu items pack the command id in the wparam LOWORD;
               // wparam HIWORD is 0 for menu (vs accelerator/control).
               let cmd-id = %lo-word(wparam);
               if (cmd-id = 100)        // File → Open...
                 let new-path = %show-open-file-dialog(hwnd);
                 if (~ empty?(new-path))
                   let new-source = %read-file(new-path);
                   if (~ empty?(new-source))
                     source-text := new-source;
                     current-path := new-path;
                     buffer-lines := nod-count-newlines(new-source);
                     buffer-max-cols := nod-max-line-chars(new-source);
                     client-width-px  := buffer-max-cols * char-width;
                     client-height-px := buffer-lines * line-height;
                     scroll-x-px := 0;
                     scroll-y-px := 0;
                     %set-scroll-info(hwnd, 1, 0, client-height-px, viewport-height-px, 0, 1);
                     %set-scroll-info(hwnd, 0, 0, client-width-px,  viewport-width-px,  0, 1);
                     recent-paths := nod-add-recent(new-path, recent-paths);
                     rebuild-recent-submenu(recent-menu, recent-paths);
                     DrawMenuBar(hwnd);
                     update-title(hwnd, new-path);
                     InvalidateRect(hwnd, 0, 0);
                   else 0 end;
                 else 0 end;
                 0
               elseif (cmd-id = 101)    // File → Save
                 // If no current-path yet, fall through to Save As: pop
                 // the save dialog so the user can name the file. If
                 // we have a path, just rewrite that file with the
                 // in-memory contents (currently identical to what's
                 // on disk — Sprint 41h+ adds dirty-flag tracking).
                 if (empty?(current-path))
                   let chosen = %show-save-file-dialog(hwnd);
                   if (~ empty?(chosen))
                     let ok = %write-file(chosen, source-text);
                     if (ok = 1)
                       current-path := chosen;
                       recent-paths := nod-add-recent(chosen, recent-paths);
                       rebuild-recent-submenu(recent-menu, recent-paths);
                       DrawMenuBar(hwnd);
                       update-title(hwnd, chosen);
                     else 0 end;
                   else 0 end;
                 else
                   %write-file(current-path, source-text);
                   0
                 end;
                 0
               elseif (cmd-id = 102)    // File → Save As...
                 let chosen = %show-save-file-dialog(hwnd);
                 if (~ empty?(chosen))
                   let ok = %write-file(chosen, source-text);
                   if (ok = 1)
                     current-path := chosen;
                     recent-paths := nod-add-recent(chosen, recent-paths);
                     rebuild-recent-submenu(recent-menu, recent-paths);
                     DrawMenuBar(hwnd);
                     update-title(hwnd, chosen);
                   else 0 end;
                 else 0 end;
                 0
               elseif (cmd-id = 199)    // File → Exit
                 PostQuitMessage(0);
                 0
               elseif (cmd-id = 200)    // Help → About
                 // Sprint 41f workaround — see SetWindowTextW
                 // declaration comment above.
                 SetWindowTextW(hwnd,
                                "NewOpenDylan IDE - Sprint 41g (About)");
                 0
               elseif (cmd-id > 300 & cmd-id < 306)  // Recent items 301..305
                 // Convert 1-based menu position to 0-based list index.
                 let idx = cmd-id - 301;
                 let cursor = recent-paths;
                 let i = 0;
                 // Walk to the requested index. If the list is shorter
                 // than expected (stale menu vs. live list — shouldn't
                 // happen but defensive), `cursor` lands on nil and we
                 // bail out.
                 until (i = idx | empty?(cursor))
                   cursor := tail(cursor);
                   i := i + 1;
                 end;
                 if (~ empty?(cursor))
                   let path = head(cursor);
                   let bytes = %read-file(path);
                   if (~ empty?(bytes))
                     source-text := bytes;
                     current-path := path;
                     buffer-lines := nod-count-newlines(bytes);
                     buffer-max-cols := nod-max-line-chars(bytes);
                     client-width-px  := buffer-max-cols * char-width;
                     client-height-px := buffer-lines * line-height;
                     scroll-x-px := 0;
                     scroll-y-px := 0;
                     %set-scroll-info(hwnd, 1, 0, client-height-px, viewport-height-px, 0, 1);
                     %set-scroll-info(hwnd, 0, 0, client-width-px,  viewport-width-px,  0, 1);
                     recent-paths := nod-add-recent(path, recent-paths);
                     rebuild-recent-submenu(recent-menu, recent-paths);
                     DrawMenuBar(hwnd);
                     update-title(hwnd, path);
                     InvalidateRect(hwnd, 0, 0);
                   else 0 end;
                 else 0 end;
                 0
               else
                 // Unknown command id — defer to the OS default.
                 DefWindowProcW(hwnd, msg, wparam, lparam)
               end
             elseif (msg = 2)  // WM_DESTROY
               PostQuitMessage(0);
               0
             else
               DefWindowProcW(hwnd, msg, wparam, lparam)
             end
           end;
  let cb = as-wndproc-callback(wp);
  let atom = %register-window-class(cb, "NodIDE");
  // dwStyle = WS_OVERLAPPEDWINDOW (0xCF0000)
  //         | WS_VSCROLL          (0x00200000)
  //         | WS_HSCROLL          (0x00100000)
  //         = 16711680.
  // hMenu = `menu-bar` HMENU (10th arg).
  let hwnd = CreateWindowExW(0, atom, "NewOpenDylan IDE",
                             16711680, -2147483648, -2147483648, 1024, 768,
                             0, menu-bar, 0, 0);
  swap := %dxgi-create-swap-chain-for-hwnd(dxgi-factory, d3d-device, hwnd, 1024, 768);
  %set-scroll-info(hwnd, 1, 0, client-height-px, viewport-height-px, 0, 1);
  %set-scroll-info(hwnd, 0, 0, client-width-px,  viewport-width-px,  0, 1);
  update-title(hwnd, current-path);
  ShowWindow(hwnd, 5);
  UpdateWindow(hwnd);
  %run-message-loop();
end function main;
