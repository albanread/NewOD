Module: nod-ide

// Sprint 41e — menu bar (File / Help) on top of Sprint 41d's corrected
// editor model.
//
// Sprint 41d delivered the buffer-sized canvas + both scrollbars; Sprint
// 41e adds the missing "Windows app" feel:
//
//   File
//     Open... Ctrl+O   (cmd-id 100)
//     ───────
//     Exit    Alt+F4   (cmd-id 199)
//   Help
//     About            (cmd-id 200)
//
// Three commands:
//   * File → Open    — pops the system file-open dialog via
//                      `%show-open-file-dialog`, then reloads the
//                      source buffer, recomputes the buffer dims, resets
//                      both scroll offsets, reconfigures the scrollbars,
//                      and invalidates the window for repaint.
//   * File → Exit    — `PostQuitMessage(0)` (same as clicking X).
//   * Help → About   — `MessageBoxW` with version + copyright.
//
// The menu bar is built via bare-name `CreateMenu` / `CreatePopupMenu` /
// `AppendMenuW` calls (Sprint 40d's bare-name materialization). The
// HMENU is passed to `CreateWindowExW` as the `hMenu` arg (10th arg,
// previously 0). `WM_COMMAND` (msg = 273 = 0x111) handles the menu
// clicks; `wparam`'s low 16 bits carry the command id.
//
// Sprint 41d follow-up (NOT addressed here): when the window is resized
// LARGER than the canvas (e.g. open a tiny file then maximize), the
// area beyond the buffer-sized canvas shows the system background color
// (often black). Fix needs canvas floor logic — track the largest
// window ever made and stretch the canvas's white background fill to
// fill that area. Deferred to a Sprint 41f or Sprint 42 polish slot.

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

// Sprint 41e — explicit declarations for the menu APIs. We avoid the
// bare-name materialization path for these three because:
//   * `AppendMenuW`'s 4th arg `lpNewItem` is typed `WideString` in the
//     vendored windows_api.db, but for MF_POPUP submenus we'd want to
//     pass an HMENU integer. Declaring it as `<c-wide-string>` keeps
//     the marshaling consistent across menu-item AND submenu calls (we
//     always pass a label string, never a popup HMENU via lpNewItem —
//     the HMENU goes in the 3rd arg `uIDNewItem`, which we type as
//     `<c-pointer>` to accept both an integer id and a submenu HMENU).
//   * Same rationale as the existing `CreateWindowExW` declaration —
//     better one fixed declaration per API than relying on the bare-
//     name path to pick the right marshaling for every overloaded arg.
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

// MessageBoxW from inside our WNDPROC HANGS — verified via diagnostic
// instrumentation: format-out before the call fires, format-out after
// never does. Cause is not yet diagnosed; Agent A (Win32 docs) said
// "trampoline re-entry unsafe" but Agent B (internal audit) said the
// trampolines ARE re-entry-safe (mutex dropped before invoke, TempBuf
// stack-scoped, GC roots pinned). Standalone MessageBoxW from main()
// (Sprint 39c test) works fine — only the WNDPROC-callback context
// fails. Filed in DEFERRED.md as Sprint 41-known-issue. Help > About
// uses SetWindowTextW as a workaround for now — it still demonstrates
// menu dispatch works without depending on the modal-dialog path.
define c-function SetWindowTextW
  (hwnd :: <c-pointer>, lpString :: <c-wide-string>)
 => (success :: <c-bool>);
    library: "user32.dll";
end;

// Sprint 41f — probe declarations stripped. The Win32 probes
// (Sleep, GetTickCount, IsWindow, EnableWindow) all completed cleanly
// in the WM_COMMAND re-entry context, demonstrating Sprint 32's
// trampoline IS re-entry-safe. See docs/duim-research/07-probe-findings.md
// for the full investigation transcript. MessageBoxW declaration kept
// in case a future TaskDialogIndirect-or-custom-popup sprint wants it
// as a fallback baseline.
define c-function MessageBoxW
  (hwnd :: <c-pointer>, lpText :: <c-wide-string>, lpCaption :: <c-wide-string>,
   uType :: <c-int>)
 => (result :: <c-int>);
    library: "user32.dll";
end;

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
  let d3d-device   = %d3d11-create-device();
  let dxgi-factory = %dxgi-factory-from-d3d-device(d3d-device);
  let dxgi-device  = %dxgi-device-from-d3d-device(d3d-device);
  let d2d-factory  = %d2d-create-factory();
  let d2d-device   = %d2d-create-device(d2d-factory, dxgi-device);
  let dc           = %d2d-create-device-context(d2d-device);
  let dwrite       = %dwrite-create-factory();
  let format       = %dwrite-create-text-format(dwrite, "Consolas", 1400, "en-us");
  // Buffer dimensions — Sprint 41e promotes these from let-bindings to
  // assignable cells (the WM_COMMAND handler reloads the buffer when
  // File → Open succeeds). Cell promotion is automatic since they're
  // captured by the WNDPROC closure and assigned inside it.
  let buffer-lines    = %count-newlines(source-text);
  let buffer-max-cols = %max-line-chars(source-text);
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
             elseif (msg = 273)  // WM_COMMAND — Sprint 41e menu dispatch
               // Menu items pack the command id in the wparam LOWORD;
               // wparam HIWORD is 0 for menu (vs accelerator/control).
               let cmd-id = %lo-word(wparam);
               if (cmd-id = 100)        // File → Open...
                 let new-path = %show-open-file-dialog(hwnd);
                 if (~ empty?(new-path))
                   let new-source = %read-file(new-path);
                   if (~ empty?(new-source))
                     // Swap in the new buffer + recompute dims + reset
                     // scroll offsets. The next WM_PAINT picks up the
                     // mutated `source-text` via the wp closure's
                     // automatically-promoted cell, and the next
                     // CreateTextLayout call uses the new buffer +
                     // client dims. No need to release any cached
                     // layout — Sprint 41d's WM_PAINT creates+releases
                     // one per frame already.
                     source-text := new-source;
                     buffer-lines := %count-newlines(new-source);
                     buffer-max-cols := %max-line-chars(new-source);
                     client-width-px  := buffer-max-cols * char-width;
                     client-height-px := buffer-lines * line-height;
                     scroll-x-px := 0;
                     scroll-y-px := 0;
                     %set-scroll-info(hwnd, 1, 0, client-height-px, viewport-height-px, 0, 1);
                     %set-scroll-info(hwnd, 0, 0, client-width-px,  viewport-width-px,  0, 1);
                     InvalidateRect(hwnd, 0, 0);
                   else 0 end;
                 else 0 end;
                 0
               elseif (cmd-id = 199)    // File → Exit
                 PostQuitMessage(0);
                 0
               elseif (cmd-id = 200)    // Help → About
                 // Sprint 41f investigated MessageBoxW-from-WNDPROC
                 // thoroughly. Findings (see docs/duim-research/
                 // 07-probe-findings.md):
                 //   * Sprint 32 trampoline IS re-entry-safe — 5 baseline
                 //     probes (Sleep, GetTickCount, IsWindow,
                 //     DefWindowProcW, EnableWindow) all complete cleanly.
                 //   * MessageBoxW with hwnd=0 + MB_TOPMOST +
                 //     MB_SETFOREGROUND DOES return cleanly (IDOK = 1)
                 //     but NO DIALOG IS VISIBLE — the OS auto-dismisses
                 //     an invisible dialog with the default response.
                 //   * The combination "DirectX-rendered window + custom
                 //     WNDPROC + modern Windows" makes MessageBox
                 //     unreliable in ways that are documented Microsoft
                 //     guidance (use TaskDialogIndirect or custom popup
                 //     instead). Foreground-lock-rule flags (TOPMOST +
                 //     SETFOREGROUND) help in normal apps but don't
                 //     rescue our DirectX-host configuration.
                 // So Help → About uses SetWindowTextW. The Sprint 41-
                 // known-issue is RESOLVED with a known-good workaround;
                 // a real modal popup waits for either TaskDialogIndirect
                 // wiring or a custom D2D-rendered popup window.
                 SetWindowTextW(hwnd,
                                "NewOpenDylan IDE - Sprint 41e (About)");
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
  // Sprint 41e — build the menu bar BEFORE CreateWindowExW so we can
  // pass the HMENU as the window's `hMenu` arg. AppendMenuW flags used:
  //   MF_STRING    = 0     (default — a plain text item)
  //   MF_POPUP     = 16    (the uIDNewItem is a submenu HMENU)
  //   MF_SEPARATOR = 2048  (horizontal divider; lpNewItem ignored)
  let menu-bar = CreateMenu();
  let file-menu = CreatePopupMenu();
  AppendMenuW(file-menu, 0,    100, "&Open...\tCtrl+O");
  AppendMenuW(file-menu, 2048, 0,   "");
  AppendMenuW(file-menu, 0,    199, "E&xit\tAlt+F4");
  AppendMenuW(menu-bar,  16,   file-menu, "&File");
  let help-menu = CreatePopupMenu();
  AppendMenuW(help-menu, 0,    200, "&About");
  AppendMenuW(menu-bar,  16,   help-menu, "&Help");
  // dwStyle = WS_OVERLAPPEDWINDOW (0xCF0000)
  //         | WS_VSCROLL          (0x00200000)
  //         | WS_HSCROLL          (0x00100000)
  //         = 16711680.
  // hMenu = `menu-bar` HMENU (10th arg, previously 0).
  let hwnd = CreateWindowExW(0, atom, "NewOpenDylan IDE",
                             16711680, -2147483648, -2147483648, 1024, 768,
                             0, menu-bar, 0, 0);
  swap := %dxgi-create-swap-chain-for-hwnd(dxgi-factory, d3d-device, hwnd, 1024, 768);
  %set-scroll-info(hwnd, 1, 0, client-height-px, viewport-height-px, 0, 1);
  %set-scroll-info(hwnd, 0, 0, client-width-px,  viewport-width-px,  0, 1);
  ShowWindow(hwnd, 5);
  UpdateWindow(hwnd);
  %run-message-loop();
end function main;
