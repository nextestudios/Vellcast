# crates/controller (`vellcast-controller`)

Entrada por gamepad do Vellcast: GilRs → ações semânticas, com deadzone configurável, histerese nos analógicos e repetição controlada. Só emite ações com a janela do app em foco (`ControllerHandle::set_active`).

Mapeamento, regras e matriz de compatibilidade: [`docs/CONTROLLER_SUPPORT.md`](../../docs/CONTROLLER_SUPPORT.md).

```rust
let handle = vellcast_controller::spawn(InputConfig::default(), |event| {
    // chamado na thread de entrada: leve o evento para a thread da UI
})?;
handle.set_active(window_focused);
```

Linux precisa de `libudev-dev` para compilar o GilRs.
