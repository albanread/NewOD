Module: nod-ide

// Sprint 41c — read-only Dylan source viewer with native-feeling
// resize + vertical scrolling.
//
// `nod-ide.exe` reads a Dylan source file path from `argv[1]` (with a
// hardcoded fallback if no arg is supplied), opens a 1024x768 window
// titled "NewOpenDylan IDE", and renders the file's contents as
// monospace text via DirectWrite. The WNDPROC handles:
//
//   * WM_PAINT       — draw the text translated by the current scroll
//                      offset, so growing the window reveals more text
//                      rather than stretching the existing pixels.
//   * WM_SIZE        — resize the swap chain back-buffer and recompute
//                      the visible-lines count for the scrollbar's
//                      proportional thumb. With DXGI_SCALING_NONE (set
//                      in `com_shim.rs::nod_dxgi_create_swap_chain_for_hwnd`)
//                      the back buffer stays at its native size between
//                      ResizeBuffers calls — exactly the Notepad++ feel.
//   * WM_VSCROLL     — user clicked the scrollbar or dragged the thumb.
//                      Update `scroll-y-line` and the OS scrollbar pos.
//   * WM_MOUSEWHEEL  — three lines per notch, standard Windows default.
//                      HIWORD is signed in Win32; we sign-extend by
//                      hand because `%hi-word` returns unsigned 0..65535.
//   * WM_KEYDOWN     — PgUp / PgDn / Home / End navigation. VK_UP /
//                      VK_DOWN / arrow keys are deferred to Sprint 41d
//                      when there's a cursor to move.
//   * WM_DESTROY     — PostQuitMessage(0).
//
// Sprint 41d will add: caret + cursor movement + arrow keys + text
// editing. Sprint 41e: undo + redo + multi-buffer.

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
  // Sprint 41c — viewport + scroll state. All captured by the WNDPROC
  // closure below and mutated via `:=`, so the Sprint 24 cell-conversion
  // pass auto-promotes them to <cell> allocations.
  let swap = 0;
  let bitmap = 0;
  let width = 1024;
  let height = 768;
  // Sprint 41c — line-height matches Consolas-14pt's nominal advance.
  // DirectWrite's actual metrics-derived line-spacing for this font/size
  // is close to 18px; a follow-up sprint can query
  // `%dwrite-get-layout-metrics` and update this dynamically.
  let line-height = 18;
  let line-count = %count-newlines(source-text);
  let scroll-y-line = 0;
  let viewport-lines = 768 / 18;  // recomputed on WM_SIZE.
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
                 let layout = %dwrite-create-text-layout(dwrite, source-text, format, width, height + scroll-y-line * line-height);
                 // Sprint 41c — draw text translated by the scroll offset.
                 // Lines above `scroll-y-line` fall off the top of the
                 // swap chain; lines below the viewport fall off the
                 // bottom. Direct2D clips both implicitly.
                 %d2d-draw-text-layout(dc, 8, 8 - scroll-y-line * line-height, layout, brush);
                 %d2d-end-draw(dc);
                 %com-release(brush);
                 %com-release(layout);
                 %dxgi-swap-chain-present(swap);
               else 0 end;
               0
             elseif (msg = 5)  // WM_SIZE
               // wparam = SIZE_MINIMIZED (1) means the window has been
               // minimised — skip resize until restored.
               if (swap ~= 0 & wparam ~= 1)
                 let new-w = %lo-word(lparam);
                 let new-h = %hi-word(lparam);
                 if (new-w > 0 & new-h > 0)
                   if (bitmap ~= 0)
                     %d2d-set-target(dc, 0);
                     %com-release(bitmap);
                     bitmap := 0;
                   else 0 end;
                   width := new-w;
                   height := new-h;
                   %dxgi-swap-chain-resize-buffers(swap, new-w, new-h);
                   // Sprint 41c — recompute the proportional thumb size
                   // for the new viewport. `nPage` in SCROLLINFO scales
                   // the thumb relative to the (max - min) range.
                   viewport-lines := new-h / line-height;
                   %set-scroll-info(hwnd, 1, 0, line-count, viewport-lines, scroll-y-line, 1);
                 else 0 end;
               else 0 end;
               0
             elseif (msg = 277)  // WM_VSCROLL
               let action = %lo-word(wparam);
               let new-pos = if (action = 0)        // SB_LINEUP
                               scroll-y-line - 1
                             elseif (action = 1)    // SB_LINEDOWN
                               scroll-y-line + 1
                             elseif (action = 2)    // SB_PAGEUP
                               scroll-y-line - (viewport-lines - 1)
                             elseif (action = 3)    // SB_PAGEDOWN
                               scroll-y-line + (viewport-lines - 1)
                             elseif (action = 4)    // SB_THUMBPOSITION
                               %hi-word(wparam)
                             elseif (action = 5)    // SB_THUMBTRACK
                               %hi-word(wparam)
                             elseif (action = 6)    // SB_TOP (Home)
                               0
                             elseif (action = 7)    // SB_BOTTOM (End)
                               line-count - viewport-lines
                             else
                               scroll-y-line       // SB_ENDSCROLL / unknown
                             end;
               // Clamp to [0, max(0, line-count - viewport-lines)].
               let max-scroll = if (line-count > viewport-lines)
                                  line-count - viewport-lines
                                else 0 end;
               let clamped = if (new-pos < 0) 0
                             elseif (new-pos > max-scroll) max-scroll
                             else new-pos end;
               if (clamped ~= scroll-y-line)
                 scroll-y-line := clamped;
                 %set-scroll-info(hwnd, 1, 0, line-count, viewport-lines, clamped, 1);
                 InvalidateRect(hwnd, 0, 0);
               else 0 end;
               0
             elseif (msg = 522)  // WM_MOUSEWHEEL
               // HIWORD(wparam) is signed in Win32; `%hi-word` returns
               // unsigned 0..65535, so sign-extend by hand.
               let raw-delta = %hi-word(wparam);
               let signed-delta = if (raw-delta > 32767)
                                    raw-delta - 65536
                                  else
                                    raw-delta
                                  end;
               // 3 lines per notch (WHEEL_DELTA=120) is the Windows default.
               // Positive delta = wheel away from user = scroll up.
               let lines-to-scroll = -1 * signed-delta * 3 / 120;
               let new-pos = scroll-y-line + lines-to-scroll;
               let max-scroll = if (line-count > viewport-lines)
                                  line-count - viewport-lines
                                else 0 end;
               let clamped = if (new-pos < 0) 0
                             elseif (new-pos > max-scroll) max-scroll
                             else new-pos end;
               if (clamped ~= scroll-y-line)
                 scroll-y-line := clamped;
                 %set-scroll-info(hwnd, 1, 0, line-count, viewport-lines, clamped, 1);
                 InvalidateRect(hwnd, 0, 0);
               else 0 end;
               0
             elseif (msg = 256)  // WM_KEYDOWN
               let vk = %lo-word(wparam);
               let max-scroll = if (line-count > viewport-lines)
                                  line-count - viewport-lines
                                else 0 end;
               let new-pos = if (vk = 33)        // VK_PRIOR (PgUp)
                               scroll-y-line - (viewport-lines - 1)
                             elseif (vk = 34)    // VK_NEXT (PgDn)
                               scroll-y-line + (viewport-lines - 1)
                             elseif (vk = 36)    // VK_HOME
                               0
                             elseif (vk = 35)    // VK_END
                               max-scroll
                             else
                               scroll-y-line
                             end;
               let clamped = if (new-pos < 0) 0
                             elseif (new-pos > max-scroll) max-scroll
                             else new-pos end;
               if (clamped ~= scroll-y-line)
                 scroll-y-line := clamped;
                 %set-scroll-info(hwnd, 1, 0, line-count, viewport-lines, clamped, 1);
                 InvalidateRect(hwnd, 0, 0);
               else 0 end;
               0
             elseif (msg = 2)  // WM_DESTROY
               PostQuitMessage(0);
               0
             else
               DefWindowProcW(hwnd, msg, wparam, lparam)
             end
           end;
  let cb = as-wndproc-callback(wp);
  let atom = %register-window-class(cb, "NodIDE");
  // Sprint 41c — dwStyle = WS_OVERLAPPEDWINDOW (0xCF0000 = 13565952)
  //                     | WS_VSCROLL          (0x00200000 = 2097152)
  //                     = 15663104.
  // The scrollbar appears on the right edge of the window; its range +
  // thumb size are configured below via `%set-scroll-info`.
  let hwnd = CreateWindowExW(0, atom, "NewOpenDylan IDE",
                             15663104, -2147483648, -2147483648, 1024, 768,
                             0, 0, 0, 0);
  swap := %dxgi-create-swap-chain-for-hwnd(dxgi-factory, d3d-device, hwnd, 1024, 768);
  // Sprint 41c — initial scrollbar config. nbar=1 (SB_VERT), min=0,
  // max=line-count, page=viewport-lines (drives the proportional thumb),
  // pos=0 (top of file), redraw=1.
  %set-scroll-info(hwnd, 1, 0, line-count, viewport-lines, 0, 1);
  ShowWindow(hwnd, 5);
  UpdateWindow(hwnd);
  %run-message-loop();
end function main;
