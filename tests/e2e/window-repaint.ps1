# Regressão da issue #6 (janela transparente após uso prolongado), no Windows.
#
# Abre o app desktop, captura a área cliente da janela como o usuário a vê (cópia
# da tela, com a janela por cima de tudo) e repete depois de cada transição que
# pode fazer o Windows descartar o conteúdo da janela: minimizar/restaurar,
# ocultar/mostrar por fora do Slint (o sistema descarta o que foi desenhado numa
# janela oculta, como acontece quando a tela dorme ou o DWM reinicia), fechar
# para a bandeja e reabrir pelo ícone, invalidação pelo sistema e espera ociosa.
# Cada captura precisa ser quase idêntica à inicial; a janela transparente ou
# vazia mostra a área de trabalho ou preto e reprova o teste.
#
# Uso: pwsh tests/e2e/window-repaint.ps1 <vellcast-desktop.exe> <pasta-de-saída> [segundos-ociosos=30]

param(
  [Parameter(Mandatory = $true)] [string] $Exe,
  [Parameter(Mandatory = $true)] [string] $OutDir,
  [int] $IdleSeconds = 30
)

$ErrorActionPreference = 'Stop'
# PowerShell 7 (.NET): Bitmap fica em System.Drawing.Common.
Add-Type -AssemblyName System.Drawing.Common
Add-Type -ReferencedAssemblies System.Drawing.Common, System.Drawing.Primitives -TypeDefinition @"
using System; using System.Text; using System.Drawing; using System.Drawing.Imaging;
using System.Runtime.InteropServices;
public static class Wnd {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool InvalidateRect(IntPtr h, IntPtr r, bool erase);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);

  // Primeira janela do processo com a classe pedida (visível ou não).
  public static IntPtr Find(uint target, string cls) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => {
      uint pid; GetWindowThreadProcessId(h, out pid);
      if (pid != target) return true;
      var c = new StringBuilder(256); GetClassName(h, c, 256);
      var t = new StringBuilder(256); GetWindowText(h, t, 256);
      bool match = cls == "" ? (t.ToString() == "Vellcast" && c.ToString() != "tray_icon_app") : c.ToString() == cls;
      if (match) { found = h; return false; }
      return true;
    }, IntPtr.Zero);
    return found;
  }

  // Coloca a janela acima de todas as outras (TOPMOST, NOSIZE|NOMOVE|NOACTIVATE).
  public static void Raise(IntPtr h) { SetWindowPos(h, new IntPtr(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010); }

  // Copia da tela a área cliente da janela, como o usuário a vê.
  public static Bitmap Grab(IntPtr h) {
    RECT r; GetClientRect(h, out r);
    POINT p = new POINT(); ClientToScreen(h, ref p);
    int w = Math.Max(1, r.R - r.L), hh = Math.Max(1, r.B - r.T);
    var bmp = new Bitmap(w, hh, PixelFormat.Format32bppArgb);
    using (var g = Graphics.FromImage(bmp)) g.CopyFromScreen(p.X, p.Y, 0, 0, new Size(w, hh));
    return bmp;
  }

  // Fração de pixels (amostrados) iguais aos da referência, com tolerância por canal.
  public static double Similarity(Bitmap a, Bitmap b) {
    if (a.Width != b.Width || a.Height != b.Height) return 0;
    long same = 0, total = 0;
    for (int y = 0; y < a.Height; y += 3)
      for (int x = 0; x < a.Width; x += 3) {
        Color c1 = a.GetPixel(x, y), c2 = b.GetPixel(x, y);
        total++;
        if (Math.Abs(c1.R - c2.R) <= 8 && Math.Abs(c1.G - c2.G) <= 8 && Math.Abs(c1.B - c2.B) <= 8) same++;
      }
    return total == 0 ? 0 : (double)same / total;
  }

  // Fração de pixels com o fundo do app (Theme.bg-0 = #0E1014).
  public static double AppBackground(Bitmap a) {
    long hit = 0, total = 0;
    for (int y = 0; y < a.Height; y += 3)
      for (int x = 0; x < a.Width; x += 3) {
        Color c = a.GetPixel(x, y);
        total++;
        if (Math.Abs(c.R - 0x0E) <= 3 && Math.Abs(c.G - 0x10) <= 3 && Math.Abs(c.B - 0x14) <= 3) hit++;
      }
    return total == 0 ? 0 : (double)hit / total;
  }
}
"@

function Capture([IntPtr] $h) {
  [Wnd]::Raise($h)
  Start-Sleep -Milliseconds 700
  return [Wnd]::Grab($h)
}

New-Item -ItemType Directory -Force $OutDir | Out-Null
$env:RUST_LOG = 'info,vellcast_desktop=debug'
$p = Start-Process $Exe -PassThru
$failed = @()
try {
  Start-Sleep -Seconds 8
  $hwnd = [Wnd]::Find([uint32]$p.Id, '')
  if ($hwnd -eq [IntPtr]::Zero) { throw 'janela principal do Vellcast não encontrada' }

  $base = Capture $hwnd
  $base.Save((Join-Path $OutDir '0-inicial.png'))
  $bg = [Wnd]::AppBackground($base)
  Write-Output ("inicial: {0}x{1}, fundo do app em {2:P1} da área" -f $base.Width, $base.Height, $bg)
  if ($bg -lt 0.10) { throw 'a captura inicial não parece o app (fundo do app ausente)' }

  function Check([string] $name) {
    Start-Sleep -Seconds 2
    $img = Capture $hwnd
    $img.Save((Join-Path $OutDir "$name.png"))
    $sim = [Wnd]::Similarity($base, $img)
    $line = "{0}: {1:P1} igual à inicial, fundo do app em {2:P1}" -f $name, $sim, [Wnd]::AppBackground($img)
    Write-Output $line
    if ($sim -lt 0.85) { $script:failed += $line }
  }

  # Minimizar e restaurar.
  [Wnd]::ShowWindow($hwnd, 6) | Out-Null; Start-Sleep -Seconds 2
  [Wnd]::ShowWindow($hwnd, 9) | Out-Null
  Check '1-minimizar-restaurar'

  # Ocultar e mostrar por fora do Slint: o Windows descarta o conteúdo da janela
  # e pede repintura com região inválida (o mesmo caminho da tela que dorme).
  [Wnd]::ShowWindow($hwnd, 0) | Out-Null; Start-Sleep -Seconds 2
  [Wnd]::ShowWindow($hwnd, 5) | Out-Null
  Check '2-ocultar-mostrar-sistema'

  # Fechar para a bandeja (padrão) e reabrir com clique no ícone.
  $tray = [Wnd]::Find([uint32]$p.Id, 'tray_icon_app')
  if ($tray -ne [IntPtr]::Zero) {
    [Wnd]::PostMessage($hwnd, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null  # WM_CLOSE
    Start-Sleep -Seconds 2
    if ([Wnd]::IsWindowVisible($hwnd)) { $failed += 'fechar não ocultou a janela na bandeja' }
    # WM_USER_TRAYICON (6002) com WM_LBUTTONUP (0x0202): clique no ícone.
    [Wnd]::PostMessage($tray, 6002, [IntPtr]::Zero, [IntPtr]0x0202) | Out-Null
    Check '3-bandeja-ocultar-mostrar'
  } else { Write-Output 'sem ícone de bandeja nesta sessão; etapa da bandeja pulada' }

  # Invalidação pelo sistema sem mudança de visibilidade.
  [Wnd]::InvalidateRect($hwnd, [IntPtr]::Zero, $true) | Out-Null
  Check '4-invalidacao'

  # Ociosa por um tempo, depois minimizar/restaurar de novo.
  Start-Sleep -Seconds $IdleSeconds
  Check '5-ociosa'
  [Wnd]::ShowWindow($hwnd, 0) | Out-Null; Start-Sleep -Seconds 1
  [Wnd]::ShowWindow($hwnd, 5) | Out-Null
  Check '6-ociosa-ocultar-mostrar'
}
finally {
  Stop-Process -Id $p.Id -ErrorAction SilentlyContinue
}

$summary = if ($failed.Count -eq 0) { 'janela renderizada por inteiro em todas as transições' } else { "falhas: $($failed -join '; ')" }
if ($env:GITHUB_STEP_SUMMARY) { "### App desktop — repintura da janela (issue #6)`n`n$summary" | Out-File -Append $env:GITHUB_STEP_SUMMARY }
if ($failed.Count -gt 0) { Write-Output $summary; exit 1 }
Write-Output $summary
