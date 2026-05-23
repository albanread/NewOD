Module: nod-ide

// Sprint 41b — read-only Dylan source viewer.
//
// `nod-ide.exe` reads a Dylan source file path from `argv[1]` (with a
// hardcoded fallback if no arg is supplied), opens a 1024x768 window
// titled "NewOpenDylan IDE", and renders the file's contents as
// monospace text via DirectWrite. The WNDPROC handles WM_SIZE so
// dragging the window edge re-renders the text at the new viewport
// size without artifacts.
//
// This is the same Dylan source body the `aot_nod_ide_source_viewer`
// test in `tests/nod-tests/tests/aot_ide_shell.rs` embeds verbatim. It
// lives here as a stand-alone fixture so the test framework can
// reference it and so a future contributor can `nod-driver build` it
// directly to play with the IDE shell.
//
// Sprint 41c will add: cached brush (avoid re-creating per WM_PAINT),
// scrollbar handling, line-number gutter, syntax highlighting via
// `nod-reader`.

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
  let swap = 0;
  let bitmap = 0;
  let width = 1024;
  let height = 768;
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
                 let layout = %dwrite-create-text-layout(dwrite, source-text, format, width, height);
                 %d2d-draw-text-layout(dc, 8, 8, layout, brush);
                 %d2d-end-draw(dc);
                 %com-release(brush);
                 %com-release(layout);
                 %dxgi-swap-chain-present(swap);
               else 0 end;
               0
             elseif (msg = 5)  // WM_SIZE
               // wparam = SIZE_MINIMIZED (1) means the window has been
               // minimised — DXGI's ResizeBuffers fails on zero
               // dimensions, so skip the resize until the window
               // restores. wparam = SIZE_RESTORED (0) / SIZE_MAXIMIZED
               // (2) both bring real dimensions in lparam's low/high
               // 16 bits.
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
  let hwnd = CreateWindowExW(0, atom, "NewOpenDylan IDE",
                             13565952, -2147483648, -2147483648, 1024, 768,
                             0, 0, 0, 0);
  swap := %dxgi-create-swap-chain-for-hwnd(dxgi-factory, d3d-device, hwnd, 1024, 768);
  ShowWindow(hwnd, 5);
  UpdateWindow(hwnd);
  %run-message-loop();
end function main;
