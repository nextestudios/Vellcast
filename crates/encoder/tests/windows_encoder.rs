//! Testes do encoder no Windows real.
//!
//! O teste de codificação precisa de uma GPU (ou do adaptador básico) com
//! suporte a processamento de vídeo D3D11 e de um encoder H.264 do Media
//! Foundation. Runners de CI sem GPU normalmente não têm isso, por isso ele é
//! `#[ignore]`: rode na máquina de teste com
//! `cargo test -p vellcast-encoder -- --ignored`.
#![cfg(windows)]
#![allow(unsafe_code)]

use std::time::{Duration, Instant};

use vellcast_encoder::annexb::{NAL_IDR, NAL_PPS, NAL_SPS, nal_type, nal_units};
use vellcast_encoder::{
    EncodedFrame, EncoderConfig, H264Encoder, SubmitOutcome, init_thread, list_encoders,
};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG,
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11CreateDevice, ID3D11Device, ID3D11Multithread,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::core::Interface;

#[test]
fn enumeration_does_not_fail() {
    // Pode retornar lista vazia em máquinas sem Media Foundation (ex.: Windows Server).
    let list = list_encoders().expect("list_encoders");
    for e in &list {
        println!("{} {:?} hardware={}", e.name, e.vendor, e.hardware);
    }
}

fn device() -> ID3D11Device {
    let mut device = None;
    // SAFETY: parâmetros válidos; saída local.
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_FLAG(
                D3D11_CREATE_DEVICE_BGRA_SUPPORT.0 | D3D11_CREATE_DEVICE_VIDEO_SUPPORT.0,
            ),
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )
    }
    .expect("D3D11CreateDevice");
    let device = device.expect("device");
    if let Ok(mt) = device.cast::<ID3D11Multithread>() {
        // SAFETY: interface válida.
        unsafe {
            let _ = mt.SetMultithreadProtected(true);
        }
    }
    device
}

fn bgra_texture(device: &ID3D11Device, w: u32, h: u32, shade: u8) -> ID3D11Texture2D {
    let pixels: Vec<u8> = (0..w * h)
        .flat_map(|i| {
            let x = (i % w) as u8;
            [x.wrapping_add(shade), shade, (i / w) as u8, 255]
        })
        .collect();
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: pixels.as_ptr().cast(),
        SysMemPitch: w * 4,
        SysMemSlicePitch: 0,
    };
    let mut texture = None;
    // SAFETY: descritor e dados válidos durante a chamada.
    unsafe { device.CreateTexture2D(&desc, Some(&data), Some(&mut texture)) }
        .expect("CreateTexture2D");
    texture.expect("texture")
}

#[test]
#[ignore = "requer GPU/adaptador com processamento de vídeo D3D11 e encoder H.264 do Media Foundation"]
fn encodes_synthetic_frames_with_sps_idr_and_forced_keyframe() {
    init_thread();
    let device = device();
    let (w, h, fps) = (1280u32, 720u32, 30u32);
    let mut encoder = H264Encoder::new(
        &device,
        EncoderConfig {
            width: w,
            height: h,
            fps,
            bitrate_kbps: 4000,
            keyframe_interval_s: 60,
            preferred: None,
        },
    )
    .expect("encoder");
    println!("encoder: {:?}", encoder.descriptor());

    // Fonte maior que a saída e com proporção diferente: exercita escala + letterbox.
    let textures: Vec<_> = (0..4)
        .map(|i| bgra_texture(&device, 1600, 1200, i * 60))
        .collect();
    let mut out: Vec<EncodedFrame> = Vec::new();
    let frame_hns = 10_000_000 / i64::from(fps);
    let mut submitted = 0i64;
    let forced_at = 40i64;
    let deadline = Instant::now() + Duration::from_secs(20);
    while submitted < 60 && Instant::now() < deadline {
        encoder.poll(&mut out).expect("poll");
        if !encoder.ready_for_input() {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }
        let tex = &textures[(submitted % 4) as usize];
        let outcome = encoder
            .submit(
                tex,
                1600,
                1200,
                submitted * frame_hns,
                submitted == forced_at,
                &mut out,
            )
            .expect("submit");
        if outcome == SubmitOutcome::Accepted {
            submitted += 1;
        }
    }
    let drain_until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < drain_until && out.len() < 55 {
        encoder.poll(&mut out).expect("poll");
        std::thread::sleep(Duration::from_millis(5));
    }

    assert!(out.len() >= 30, "poucos quadros codificados: {}", out.len());
    let first = &out[0];
    assert!(first.keyframe, "o primeiro quadro deve ser IDR");
    let types: Vec<u8> = nal_units(&first.data).iter().map(|n| nal_type(n)).collect();
    assert_eq!(
        &types[..2],
        &[NAL_SPS, NAL_PPS],
        "SPS/PPS antes do IDR: {types:?}"
    );
    assert!(types.contains(&NAL_IDR));
    let codec = encoder.codec_string().expect("codec string");
    assert!(codec.starts_with("avc1."), "{codec}");

    let forced_pts = forced_at * frame_hns;
    let forced = out
        .iter()
        .any(|f| f.keyframe && f.pts_hns >= forced_pts && f.pts_hns <= forced_pts + 5 * frame_hns);
    assert!(
        forced,
        "keyframe sob demanda não apareceu perto do quadro {forced_at}"
    );
}
