//! Windows graphics instance configuration, applied before wgpu can initialize any driver.

use eframe::{
    NativeOptions,
    egui_wgpu::WgpuSetup,
    wgpu::{Backends, Dx12Compiler},
};

/// The start-up marker: the backend a start is trying, written before the window opens and
/// removed at its first frame. One left behind means that start crashed or hung in the driver.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub const START_MARKER: &str = "graphics-start.txt";

/// A backend's name in the marker (`WGPU_BACKEND` spelling).
pub fn backend_name(b: Backends) -> &'static str {
    if b == Backends::DX12 {
        "dx12"
    } else if b == Backends::VULKAN {
        "vulkan"
    } else if b == Backends::GL {
        "gl"
    } else {
        "other"
    }
}

/// The backends listed in a marker left behind (one name per line; unknown lines ignored). All
/// three failed: start over (a driver update may have fixed one).
pub fn failed_backends(marker: &str) -> Vec<Backends> {
    let mut out: Vec<Backends> = Vec::new();
    for line in marker.lines() {
        let b = match line.trim() {
            "dx12" => Backends::DX12,
            "vulkan" => Backends::VULKAN,
            "gl" => Backends::GL,
            _ => continue,
        };
        if !out.contains(&b) {
            out.push(b);
        }
    }
    if out.len() >= 3 { Vec::new() } else { out }
}

/// The next backend to try after `failed` (DirectX 12, then Vulkan, then OpenGL).
pub fn next_backend(failed: &[Backends]) -> Option<Backends> {
    [Backends::DX12, Backends::VULKAN, Backends::GL].into_iter().find(|b| !failed.contains(b))
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

    #[test]
    fn a_start_that_never_opened_moves_on_to_the_next_backend() {
        assert!(failed_backends("").is_empty());
        assert_eq!(failed_backends("dx12\n"), vec![Backends::DX12]);
        assert_eq!(next_backend(&failed_backends("dx12\n")), Some(Backends::VULKAN));
        assert_eq!(next_backend(&failed_backends("dx12\nvulkan\n")), Some(Backends::GL));
        // Every backend failed once: start over from DirectX 12.
        assert!(failed_backends("dx12\nvulkan\ngl\n").is_empty());
        assert_eq!(failed_backends("junk\n gl \n"), vec![Backends::GL]);
        assert_eq!(backend_name(Backends::VULKAN), "vulkan");
    }

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
