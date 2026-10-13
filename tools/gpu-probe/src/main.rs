//! GPU 서피스 진단 — 이 머신에서 **투명 창이 가능한 조합**을 실측한다.
//!
//! kmd-desktop 이 Windows 에서 불투명 창을 쓰는 이유는 "wgpu alpha mode 가
//! Opaque 로 폴백"이기 때문인데, 그것이 아키텍처(x86/ARM) 문제인지 백엔드
//! 선택 문제인지 코드만 봐서는 알 수 없다. 이 도구는 투명 창을 만들고
//! 조합별로 `surface.get_capabilities().alpha_modes` 를 그대로 출력한다.
//!
//! 조합:
//!   1. DX12 + DxgiFromHwnd  (iced 0.14 의 현재 고정 경로)
//!   2. DX12 + DxgiFromVisual (DirectComposition — wgpu 27 이 지원)
//!   3. Vulkan               (드라이버가 보고하는 값 그대로)
//!
//! 실행: cargo run --manifest-path tools/gpu-probe/Cargo.toml

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

fn probe(window: Arc<Window>) {
    println!("\n=== GPU 서피스 진단 (투명 창) ===\n");

    let combos: Vec<(&str, wgpu::Backends, Option<wgpu_types::Dx12SwapchainKind>)> = vec![
        (
            "1. DX12 + DxgiFromHwnd  (iced 현재 경로)",
            wgpu::Backends::DX12,
            Some(wgpu_types::Dx12SwapchainKind::DxgiFromHwnd),
        ),
        (
            "2. DX12 + DxgiFromVisual (DirectComposition)",
            wgpu::Backends::DX12,
            Some(wgpu_types::Dx12SwapchainKind::DxgiFromVisual),
        ),
        ("3. Vulkan", wgpu::Backends::VULKAN, None),
        (
            "4. DX12 WARP(소프트웨어) + DxgiFromVisual",
            wgpu::Backends::DX12,
            Some(wgpu_types::Dx12SwapchainKind::DxgiFromVisual),
        ),
    ];

    for (i, (label, backends, swapchain_kind)) in combos.into_iter().enumerate() {
        let force_fallback = i == 3;
        println!("--- {label}");

        let mut backend_options = wgpu::BackendOptions::default();
        if let Some(kind) = swapchain_kind {
            backend_options.dx12.presentation_system = kind;
        }

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            backend_options,
            ..Default::default()
        });

        let surface = match instance.create_surface(window.clone()) {
            Ok(s) => s,
            Err(e) => {
                println!("    서피스 생성 실패: {e}\n");
                continue;
            }
        };

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: force_fallback,
        }));

        let adapter = match adapter {
            Ok(a) => a,
            Err(e) => {
                println!("    어댑터 없음: {e}\n");
                continue;
            }
        };

        let info = adapter.get_info();
        let caps = surface.get_capabilities(&adapter);
        let transparent_ok = caps
            .alpha_modes
            .iter()
            .any(|m| !matches!(m, wgpu::CompositeAlphaMode::Opaque));

        println!("    어댑터   : {} ({:?})", info.name, info.backend);
        println!("    디바이스 : {:?} / driver={}", info.device_type, info.driver);
        println!("    alpha_modes: {:?}", caps.alpha_modes);
        println!(
            "    ▶ 투명 창 : {}",
            if transparent_ok {
                "가능 ✅"
            } else {
                "불가 ❌ (Opaque only)"
            }
        );

        // 실제 스왑체인 생성 시험 — 브랜치(iced 패치)가 고르는 순서대로
        let (device, queue) =
            match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) {
                Ok(dq) => dq,
                Err(e) => {
                    println!("    디바이스 생성 실패: {e}\n");
                    continue;
                }
            };
        for mode in [
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::PostMultiplied,
        ] {
            if !caps.alpha_modes.contains(&mode) {
                continue;
            }
            let format = caps.formats[0];
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                device.push_error_scope(wgpu::ErrorFilter::Validation);
                surface.configure(
                    &device,
                    &wgpu::SurfaceConfiguration {
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        format,
                        width: 640,
                        height: 400,
                        present_mode: wgpu::PresentMode::Fifo,
                        desired_maximum_frame_latency: 2,
                        alpha_mode: mode,
                        view_formats: vec![],
                    },
                );
                let err = pollster::block_on(device.pop_error_scope());
                if err.is_some() {
                    return Err(format!("{err:?}"));
                }
                // 프레임 하나를 실제로 받아 제출까지
                let frame = surface.get_current_texture().map_err(|e| format!("{e:?}"))?;
                let view = frame.texture.create_view(&Default::default());
                let mut enc = device.create_command_encoder(&Default::default());
                {
                    let _pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            resolve_target: None,
                            depth_slice: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        ..Default::default()
                    });
                }
                queue.submit([enc.finish()]);
                frame.present();
                Ok(())
            }));
            match result {
                Ok(Ok(())) => println!("    스왑체인 {mode:?}: 생성·표시 성공 ✅"),
                Ok(Err(e)) => println!("    스왑체인 {mode:?}: 실패 ❌ {e}"),
                Err(_) => println!("    스왑체인 {mode:?}: 패닉 ❌"),
            }
        }
        println!();
    }

    println!("=== 진단 끝 ===");
}

struct App {
    window: Option<Arc<Window>>,
    done: bool,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // kmd-desktop 과 같은 조건: 데코레이션 없는 투명 창
        let attrs = Window::default_attributes()
            .with_transparent(true)
            .with_decorations(false)
            .with_visible(false)
            .with_title("kmd gpu-probe");
        let window = Arc::new(event_loop.create_window(attrs).expect("창 생성 실패"));
        self.window = Some(window.clone());

        probe(window);
        self.done = true;
        event_loop.exit();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if matches!(event, WindowEvent::CloseRequested) {
            event_loop.exit();
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("이벤트 루프 생성 실패");
    let mut app = App {
        window: None,
        done: false,
    };
    let _ = event_loop.run_app(&mut app);
}
