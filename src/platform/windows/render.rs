//! Presentación layered: vista iced → tiny-skia → DIB BGRA → UpdateLayeredWindow.

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use std::slice;

use iced::{Color, Rectangle, Size, Theme};
use iced_runtime::core::clipboard;
use iced_runtime::core::mouse;
use iced_runtime::core::renderer;
use iced_runtime::user_interface::{self, UserInterface};
use iced_tiny_skia::graphics::Viewport;
use windows_sys::Win32::Foundation::{HWND, POINT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HDC,
    HGDIOBJ, SelectObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    HWND_TOPMOST, SW_HIDE, SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetForegroundWindow,
    SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow,
};

use super::logging;
use crate::state::Hidebar;
use crate::ui;

/// Una ventana layered con su DIB, renderer y cache de iced.
pub(crate) struct LayeredSurface {
    hwnd: HWND,
    position: POINT,
    width: u32,
    height: u32,
    scale: f32,
    mem_dc: HDC,
    dib: HBITMAP,
    old_bitmap: HGDIOBJ,
    pixels: *mut u32,
    renderer: iced_tiny_skia::Renderer,
    cache: user_interface::Cache,
    pixmap: tiny_skia::Pixmap,
    mask: tiny_skia::Mask,
}

impl LayeredSurface {
    pub(crate) fn new(hwnd: HWND, position: POINT, width: u32, height: u32, scale: f32) -> Self {
        let (mem_dc, dib, old_bitmap, pixels) = create_dib(width, height);
        let pixmap = tiny_skia::Pixmap::new(width, height).expect("Create pixmap");
        let mask = tiny_skia::Mask::new(width, height).expect("Create clip mask");

        Self {
            hwnd,
            position,
            width,
            height,
            scale,
            mem_dc,
            dib,
            old_bitmap,
            pixels,
            renderer: iced_tiny_skia::Renderer::new(iced::Font::default(), iced::Pixels(16.0)),
            cache: user_interface::Cache::default(),
            pixmap,
            mask,
        }
    }

    pub(crate) fn window(&self) -> HWND {
        self.hwnd
    }

    pub(crate) fn render(&mut self, state: &Hidebar, surface: ui::Surface, cursor: mouse::Cursor) {
        let logical = Size::new(
            self.width as f32 / self.scale,
            self.height as f32 / self.scale,
        );

        {
            let view = ui::view(state, surface);
            let mut interface = UserInterface::build(
                view,
                logical,
                std::mem::take(&mut self.cache),
                &mut self.renderer,
            );
            let style = renderer::Style {
                text_color: Color::WHITE,
            };

            interface.draw(&mut self.renderer, &Theme::Dark, &style, cursor);
            self.cache = interface.into_cache();
        }

        let viewport = Viewport::with_physical_size(Size::new(self.width, self.height), self.scale);
        let damage = Rectangle::with_size(viewport.logical_size());

        self.renderer.draw(
            &mut self.pixmap.as_mut(),
            &mut self.mask,
            &viewport,
            &[damage],
            Color::TRANSPARENT,
        );

        let source = self.pixmap.data();
        // SAFETY: `pixels` apunta al DIB de width*height u32 creado en
        // `create_dib` con estas mismas dimensiones; vive hasta `Drop`.
        let destination =
            unsafe { slice::from_raw_parts_mut(self.pixels, (self.width * self.height) as usize) };

        // tiny-skia: RGBA premultiplicado. DIB 32 bpp: BGRA premultiplicado.
        for (pixel, output) in source.as_chunks::<4>().0.iter().zip(destination.iter_mut()) {
            *output = (u32::from(pixel[3]) << 24)
                | (u32::from(pixel[0]) << 16)
                | (u32::from(pixel[1]) << 8)
                | u32::from(pixel[2]);
        }

        self.present();
    }

    /// Aplica un evento al calendario y devuelve los mensajes de la vista.
    pub(crate) fn update(
        &mut self,
        state: &Hidebar,
        event: iced::Event,
        cursor: mouse::Cursor,
    ) -> Vec<ui::Message> {
        let logical = Size::new(
            self.width as f32 / self.scale,
            self.height as f32 / self.scale,
        );
        let view = ui::view(state, ui::Surface::Calendar);
        let mut interface = UserInterface::build(
            view,
            logical,
            std::mem::take(&mut self.cache),
            &mut self.renderer,
        );

        let mut messages = Vec::new();
        let mut clipboard = clipboard::Null;
        let _ = interface.update(
            &[event],
            cursor,
            &mut self.renderer,
            &mut clipboard,
            &mut messages,
        );
        self.cache = interface.into_cache();

        messages
    }

    /// Olvida el estado de widgets (hover/pressed) al ocultar la superficie.
    pub(crate) fn reset_cache(&mut self) {
        self.cache = user_interface::Cache::default();
    }

    pub(crate) fn show(&self, activate: bool) {
        // SAFETY: `hwnd` es una ventana layered viva; solo cambia visibilidad/foco.
        if activate {
            unsafe {
                ShowWindow(self.hwnd, SW_SHOW);
                SetForegroundWindow(self.hwnd);
            }
        } else {
            unsafe {
                ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
        }
    }

    pub(crate) fn hide(&self) {
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub(crate) fn move_resize(&mut self, position: POINT, width: u32, height: u32, scale: f32) {
        if width == self.width && height == self.height && scale == self.scale {
            self.position = position;
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    position.x,
                    position.y,
                    width as i32,
                    height as i32,
                    SWP_NOACTIVATE,
                );
            }
            return;
        }

        // SAFETY: se restaura el bitmap original antes de borrar el DIB y el DC;
        // ambos handles pertenecen a esta superficie y no se usan después.
        unsafe {
            SelectObject(self.mem_dc, self.old_bitmap);
            DeleteObject(self.dib);
            DeleteDC(self.mem_dc);
        }

        let (mem_dc, dib, old_bitmap, pixels) = create_dib(width, height);
        self.mem_dc = mem_dc;
        self.dib = dib;
        self.old_bitmap = old_bitmap;
        self.pixels = pixels;
        self.pixmap = tiny_skia::Pixmap::new(width, height).expect("Create pixmap");
        self.mask = tiny_skia::Mask::new(width, height).expect("Create clip mask");
        self.position = position;
        self.width = width;
        self.height = height;
        self.scale = scale;

        unsafe {
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                position.x,
                position.y,
                width as i32,
                height as i32,
                SWP_NOACTIVATE,
            );
        }
    }

    fn present(&self) {
        let size = SIZE {
            cx: self.width as i32,
            cy: self.height as i32,
        };
        let source_point = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };

        // SAFETY: `hwnd` es layered y está viva; `size` coincide con el DIB
        // seleccionado en `mem_dc`; `position`, `source_point` y `blend` son locales.
        let updated = unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                null_mut(),
                &self.position,
                &size,
                self.mem_dc,
                &source_point,
                0,
                &blend,
                ULW_ALPHA,
            )
        };

        if updated == 0 {
            logging::error(format!(
                "hidebar: UpdateLayeredWindow falló: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
}

impl Drop for LayeredSurface {
    fn drop(&mut self) {
        // SAFETY: handles creados en `create_dib` y no liberados antes.
        unsafe {
            SelectObject(self.mem_dc, self.old_bitmap);
            DeleteObject(self.dib);
            DeleteDC(self.mem_dc);
        }
    }
}

fn create_dib(width: u32, height: u32) -> (HDC, HBITMAP, HGDIOBJ, *mut u32) {
    // SAFETY: DC compatible con pantalla; el DIB se selecciona en él y ambos
    // se liberan en `Drop`. `bits` apunta a width*height píxeles de 32 bpp.
    let mem_dc = unsafe { CreateCompatibleDC(null_mut()) };
    assert!(!mem_dc.is_null(), "CreateCompatibleDC");

    let mut bitmap_info: BITMAPINFO = unsafe { zeroed() };
    bitmap_info.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width as i32,
        biHeight: -(height as i32),
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..unsafe { zeroed() }
    };

    let mut bits: *mut c_void = null_mut();
    let dib = unsafe {
        CreateDIBSection(
            mem_dc,
            &bitmap_info,
            DIB_RGB_COLORS,
            &mut bits,
            null_mut(),
            0,
        )
    };
    assert!(!dib.is_null(), "CreateDIBSection");

    let old_bitmap = unsafe { SelectObject(mem_dc, dib) };
    assert!(!old_bitmap.is_null(), "SelectObject");

    (mem_dc, dib, old_bitmap, bits.cast())
}
