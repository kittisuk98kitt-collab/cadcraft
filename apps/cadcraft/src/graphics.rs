//! Windows graphics instance configuration, applied before wgpu can initialize any driver.

use eframe::{
    NativeOptions,
    egui_wgpu::WgpuSetup,
    wgpu::{Backends, Dx12Compiler},
};

/// The graphics backend for this machine: DirectX 12 when a graphics card supports it, else
/// Vulkan, else OpenGL as the last resort (older cards and drivers on Windows 10 have no DX12
/// device, and the window then never opened). `WGPU_BACKEND` decides when set. The probe makes a
/// window-less instance per backend and asks it for its adapters before eframe starts: the
/// window system can be started only once, so a failed start cannot be retried.
#[cfg(target_os = "windows")]
pub fn pick_backend(backend_override: Option<Backends>, compiler: Dx12Compiler) -> Backends {
    if let Some(b) = backend_override {
        return b;
    }
    let available = eframe::wgpu::Instance::enabled_backend_features();
    for b in [Backends::DX12, Backends::VULKAN] {
        if !available.contains(b) {
            continue;
        }
        let mut desc = eframe::wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = b;
        desc.backend_options.dx12.shader_compiler = compiler.clone();
        let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let instance = eframe::wgpu::Instance::new(desc);
            pollster::block_on(instance.enumerate_adapters(b)).into_iter().map(|a| a.get_info().name).collect::<Vec<_>>()
        }));
        match found {
            Ok(names) if !names.is_empty() => {
                log::info!("Windows graphics: {b:?} adapters {names:?}");
                return b;
            }
            Ok(_) => log::warn!("Windows graphics: no {b:?} adapter"),
            Err(_) => log::warn!("Windows graphics: the {b:?} probe failed"),
        }
    }
    log::warn!("Windows graphics: falling back to OpenGL");
    Backends::GL
}

/// Default to DirectX 12 with FXC before creating the wgpu instance. Explicit
/// `WGPU_BACKEND` and `WGPU_DX12_COMPILER` choices, read by the caller, take precedence.
pub fn configure(options: &mut NativeOptions, backend_override: Option<Backends>, compiler_override: Option<Dx12Compiler>) {
    if let WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        // eframe's default includes GL. Creating that backend can crash inside AMD's
        // atio6axx.dll before adapter selection or any Rust error handling runs, so the
        // window never appears. Choosing an adapter afterwards is too late: only leaving GL
        // out of the instance avoids it. This matches the verified WGPU_BACKEND=dx12
        // workaround without mutating process environment variables.
        create.instance_descriptor.backends = backend_override.unwrap_or(Backends::DX12);
        // Auto can load another program's dxcompiler.dll without a matching dxil.dll.
        // That fails wgpu's indirect-validation pipeline and loses the device before
        // the first frame (issue #608). FXC uses the Windows-provided compiler runtime.
        create.instance_descriptor.backend_options.dx12.shader_compiler = compiler_override.unwrap_or(Dx12Compiler::Fxc);
        log::info!(
            "Windows graphics: backends={:?}, DX12 compiler={:?}",
            create.instance_descriptor.backends,
            create.instance_descriptor.backend_options.dx12.shader_compiler
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured_backends(backend_override: Option<Backends>) -> Backends {
        let mut options = NativeOptions::default();
        configure(&mut options, backend_override, None);
        let WgpuSetup::CreateNew(create) = options.wgpu_options.wgpu_setup else {
            panic!("default native options must create a wgpu instance");
        };
        create.instance_descriptor.backends
    }

    #[test]
    fn windows_default_initializes_only_dx12() {
        // eframe's own default (PRIMARY | GL) would initialize OpenGL as well.
        let WgpuSetup::CreateNew(eframe_default) = NativeOptions::default().wgpu_options.wgpu_setup else {
            panic!("default native options must create a wgpu instance");
        };
        assert!(eframe_default.instance_descriptor.backends.contains(Backends::GL));
        assert_eq!(configured_backends(None), Backends::DX12);
    }

    #[test]
    fn windows_default_uses_fxc_instead_of_searching_for_dxc() {
        let mut options = NativeOptions::default();
        if let WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
            // Auto can find an unrelated DXC without its matching DXIL runtime (issue #608).
            create.instance_descriptor.backend_options.dx12.shader_compiler = Dx12Compiler::Auto;
        }
        configure(&mut options, None, None);
        let WgpuSetup::CreateNew(create) = options.wgpu_options.wgpu_setup else {
            panic!("default native options must create a wgpu instance");
        };
        assert!(matches!(create.instance_descriptor.backend_options.dx12.shader_compiler, Dx12Compiler::Fxc));
    }

    #[test]
    fn explicit_compiler_and_backend_overrides_are_preserved() {
        let backends = Backends::VULKAN | Backends::DX12;
        for compiler in
            [Dx12Compiler::Fxc, Dx12Compiler::Auto, Dx12Compiler::StaticDxc, Dx12Compiler::DynamicDxc { dxc_path: "custom/dxcompiler.dll".into() }]
        {
            let mut options = NativeOptions::default();
            configure(&mut options, Some(backends), Some(compiler.clone()));
            let WgpuSetup::CreateNew(create) = options.wgpu_options.wgpu_setup else {
                panic!("default native options must create a wgpu instance");
            };
            assert_eq!(create.instance_descriptor.backends, backends);
            match (compiler, create.instance_descriptor.backend_options.dx12.shader_compiler) {
                (Dx12Compiler::Fxc, Dx12Compiler::Fxc)
                | (Dx12Compiler::Auto, Dx12Compiler::Auto)
                | (Dx12Compiler::StaticDxc, Dx12Compiler::StaticDxc) => {}
                (Dx12Compiler::DynamicDxc { dxc_path: expected }, Dx12Compiler::DynamicDxc { dxc_path: actual }) => {
                    assert_eq!(actual, expected);
                }
                (expected, actual) => panic!("compiler override changed from {expected:?} to {actual:?}"),
            }
        }
    }

    #[test]
    fn explicit_backend_override_is_preserved() {
        for backends in [Backends::VULKAN, Backends::GL, Backends::DX12, Backends::VULKAN | Backends::DX12, Backends::empty()] {
            assert_eq!(configured_backends(Some(backends)), backends);
        }
    }
}
