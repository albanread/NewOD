Module: nod-ide

// Sprint 41d — corrected editor model with horizontal scrolling.
//
// The 41c viewer treated the rendered canvas as window-sized, so lines
// wider than the window were silently clipped with no way to recover.
// Sprint 41d models three distinct layers:
//
//   1. Text buffer (the file's bytes). The source of truth. Resizing
//      the window doesn't change it.
//   2. Client area (the rendered canvas, a virtual coordinate space).
//      Bounded by the BUFFER:
//        client-width-px  = buffer-max-cols  * char-width
//        client-height-px = buffer-lines     * line-height
//      Resizing the OS window NEVER changes this.
//   3. Window (the visible HWND viewport). Whatever the user dragged.
//      Shows a subset of the client area offset by
//      (scroll-x-px, scroll-y-px).
//
// The vertical scrollbar appears when client.height_px > window.height_px.
// The horizontal scrollbar appears when client.width_px > window.width_px.
// Both ranges are in PIXELS — same coordinate system as the
// drawing translation, so no unit-conversion bugs.
//
// WNDPROC messages handled:
//   * WM_PAINT      — draw the text translated by both scroll offsets.
//   * WM_SIZE       — resize swap chain back-buffer; recompute the two
//                     viewport pixel dims and reconfigure BOTH scrollbars'
//                     proportional thumbs. Client area dimensions do NOT
//                     change on resize — that's the entire point.
//   * WM_VSCROLL    — vertical scroll, in pixels.
//   * WM_HSCROLL    — horizontal scroll, in pixels (msg=276=0x114).
//   * WM_MOUSEWHEEL — vertical scroll; Shift+Wheel = horizontal scroll.
//   * WM_KEYDOWN    — PgUp/PgDn/Home/End (vertical), Left/Right (horizontal).
//                     Up/Down still deferred to a later sprint when the
//                     cursor lands.
//   * WM_DESTROY    — PostQuitMessage(0).

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
  // Sprint 41d — buffer dimensions. The source of truth. These don't
  // change on window resize.
  let buffer-lines    = %count-newlines(source-text);
  let buffer-max-cols = %max-line-chars(source-text);
  // Sprint 41d — Consolas-14pt cell size. Approximate (DirectWrite's
  // actual metrics would be ~7.4 wide / ~18 tall); a follow-up can
  // query `%dwrite-get-layout-metrics` and update these dynamically.
  let char-width  = 8;
  let line-height = 18;
  let pad = 8;
  // Sprint 41d — client area = buffer-sized canvas, in PIXELS. Bounded
  // by the buffer, NOT the window. Resizing the OS window doesn't
  // change these.
  let client-width-px  = buffer-max-cols * char-width;
  let client-height-px = buffer-lines * line-height;
  // Sprint 41d — viewport size (the visible HWND), in PIXELS. Updated
  // on every WM_SIZE.
  let window-width    = 1024;
  let window-height   = 768;
  let viewport-width-px  = 1024;
  let viewport-height-px = 768;
  // Sprint 41d — scroll offsets, in PIXELS. Same coordinate system as
  // the drawing translation below: drawing happens at
  // (pad - scroll-x-px, pad - scroll-y-px).
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
                 // Sprint 41d — layout box sized to the CLIENT AREA, not
                 // the window. DirectWrite lays out the whole buffer
                 // once; D2D clips whatever falls off the viewport.
                 let layout = %dwrite-create-text-layout(dwrite, source-text, format,
                                                         client-width-px, client-height-px);
                 // Sprint 41d — translate by BOTH scroll offsets. With
                 // scroll-x-px = scroll-y-px = 0 the file's top-left
                 // corner sits at (pad, pad).
                 %d2d-draw-text-layout(dc, pad - scroll-x-px, pad - scroll-y-px, layout, brush);
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
                   window-width  := new-w;
                   window-height := new-h;
                   viewport-width-px  := new-w;
                   viewport-height-px := new-h;
                   %dxgi-swap-chain-resize-buffers(swap, new-w, new-h);
                   // Sprint 41d — reconfigure BOTH scrollbars. Client area
                   // dimensions DO NOT change here — that's the corrected
                   // model. Only the viewport (window) sizes do.
                   //
                   // nMax is the canvas size; nPage is the viewport size in
                   // the same units, which drives the proportional thumb.
                   %set-scroll-info(hwnd, 1, 0, client-height-px, viewport-height-px, scroll-y-px, 1);
                   %set-scroll-info(hwnd, 0, 0, client-width-px,  viewport-width-px,  scroll-x-px, 1);
                 else 0 end;
               else 0 end;
               0
             elseif (msg = 277)  // WM_VSCROLL
               let action = %lo-word(wparam);
               // Sprint 41d — scroll deltas in PIXELS. SB_LINE moves one
               // line of text; SB_PAGE moves one viewport's worth.
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
             elseif (msg = 276)  // WM_HSCROLL — Sprint 41d
               let action = %lo-word(wparam);
               // Sprint 41d — horizontal deltas. SB_LINELEFT/RIGHT (0/1) use
               // one char-width; SB_PAGELEFT/RIGHT (2/3) use one viewport
               // width (minus one char so the user always sees an
               // overlap). Action codes are identical to WM_VSCROLL.
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
               // HIWORD(wparam) is signed in Win32; `%hi-word` returns
               // unsigned 0..65535, so sign-extend by hand.
               let raw-delta = %hi-word(wparam);
               let signed-delta = if (raw-delta > 32767)
                                    raw-delta - 65536
                                  else
                                    raw-delta
                                  end;
               // Sprint 41d — Shift+MouseWheel = horizontal scroll. The
               // wparam LOWORD packs Win32 modifier flags; MK_SHIFT = 4.
               // Bit-test bit 2 via integer division (which is what `/`
               // does on fixnums here): `(flags / 4) - (flags / 8) * 2`
               // is bit 2 of flags. `%logand` isn't a primitive yet.
               let flags = %lo-word(wparam);
               let shift-bit = (flags / 4) - (flags / 8) * 2;
               if (shift-bit = 1)
                 // Horizontal scroll. Positive delta = wheel away from
                 // user = scroll left (matches IE / Edge / Notepad++).
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
                 // Vertical scroll. 3 lines per notch (WHEEL_DELTA=120)
                 // is the Windows default. Positive delta = wheel away
                 // from user = scroll up (toward smaller scroll-y-px).
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
               elseif (vk = 37)    // VK_LEFT — Sprint 41d
                 let new-pos = scroll-x-px - char-width;
                 let clamped = if (new-pos < 0) 0
                               elseif (new-pos > h-max) h-max
                               else new-pos end;
                 if (clamped ~= scroll-x-px)
                   scroll-x-px := clamped;
                   %set-scroll-info(hwnd, 0, 0, client-width-px, viewport-width-px, clamped, 1);
                   InvalidateRect(hwnd, 0, 0);
                 else 0 end;
               elseif (vk = 39)    // VK_RIGHT — Sprint 41d
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
             elseif (msg = 2)  // WM_DESTROY
               PostQuitMessage(0);
               0
             else
               DefWindowProcW(hwnd, msg, wparam, lparam)
             end
           end;
  let cb = as-wndproc-callback(wp);
  let atom = %register-window-class(cb, "NodIDE");
  // Sprint 41d — dwStyle = WS_OVERLAPPEDWINDOW (0xCF0000 = 13565952)
  //                     | WS_VSCROLL          (0x00200000 = 2097152)
  //                     | WS_HSCROLL          (0x00100000 = 1048576)
  //                     = 16711680.
  // Both scrollbars appear; ranges + thumb sizes are configured below
  // via `%set-scroll-info` for SB_VERT (nbar=1) and SB_HORZ (nbar=0).
  let hwnd = CreateWindowExW(0, atom, "NewOpenDylan IDE",
                             16711680, -2147483648, -2147483648, 1024, 768,
                             0, 0, 0, 0);
  swap := %dxgi-create-swap-chain-for-hwnd(dxgi-factory, d3d-device, hwnd, 1024, 768);
  // Sprint 41d — initial scrollbar config. Both axes in pixel units.
  %set-scroll-info(hwnd, 1, 0, client-height-px, viewport-height-px, 0, 1);
  %set-scroll-info(hwnd, 0, 0, client-width-px,  viewport-width-px,  0, 1);
  ShowWindow(hwnd, 5);
  UpdateWindow(hwnd);
  %run-message-loop();
end function main;
