# Maaler om der FAKTISK loeber lyd i Quest-mikrofonens Windows-endpoint.
# Hele COM-arbejdet ligger i C#, fordi PowerShells egen typekonvertering ikke
# kan caste et __ComObject til et [ComImport]-interface. Vejen er den samme som
# Chromium/WebView2 bruger under getUserMedia: WASAPI shared mode.
#
# Brug: powershell -ExecutionPolicy Bypass -File measure-mic.ps1 [-Match Oculus] [-Seconds 20]

param([string]$Match = 'Oculus', [int]$Seconds = 20)

Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;

[ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
public class MMDeviceEnumeratorComObject { }

[ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMMDeviceEnumerator {
  int EnumAudioEndpoints(int dataFlow, int stateMask, out IMMDeviceCollection devices);
  int GetDefaultAudioEndpoint(int dataFlow, int role, out IMMDevice device);
  int GetDevice(string id, out IMMDevice device);
  int RegisterEndpointNotificationCallback(IntPtr client);
  int UnregisterEndpointNotificationCallback(IntPtr client);
}

[ComImport, Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMMDeviceCollection {
  int GetCount(out uint count);
  int Item(uint index, out IMMDevice device);
}

[ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMMDevice {
  int Activate(ref Guid iid, uint clsCtx, IntPtr activationParams, [MarshalAs(UnmanagedType.IUnknown)] out object iface);
  int OpenPropertyStore(uint access, out IPropertyStore store);
  int GetId([MarshalAs(UnmanagedType.LPWStr)] out string id);
  int GetState(out uint state);
}

[StructLayout(LayoutKind.Sequential)]
public struct PropertyKey { public Guid fmtid; public int pid; }

[StructLayout(LayoutKind.Explicit)]
public struct PropVariant { [FieldOffset(0)] public short vt; [FieldOffset(8)] public IntPtr p; }

[ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IPropertyStore {
  int GetCount(out uint count);
  int GetAt(uint index, out PropertyKey key);
  int GetValue(ref PropertyKey key, out PropVariant value);
  int SetValue(ref PropertyKey key, ref PropVariant value);
  int Commit();
}

[ComImport, Guid("1CB9AD4C-DBFA-4C32-B178-C2F568A703B2"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IAudioClient {
  int Initialize(int shareMode, int streamFlags, long bufferDuration, long periodicity, IntPtr format, IntPtr sessionGuid);
  int GetBufferSize(out uint frames);
  int GetStreamLatency(out long latency);
  int GetCurrentPadding(out uint padding);
  int IsFormatSupported(int shareMode, IntPtr format, IntPtr closestMatch);
  int GetMixFormat(out IntPtr format);
  int GetDevicePeriod(out long defaultPeriod, out long minPeriod);
  int Start();
  int Stop();
  int Reset();
  int SetEventHandle(IntPtr handle);
  int GetService(ref Guid iid, [MarshalAs(UnmanagedType.IUnknown)] out object iface);
}

[ComImport, Guid("C8ADBD64-E71E-48A0-A4DE-185C395CD317"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IAudioCaptureClient {
  int GetBuffer(out IntPtr data, out uint frames, out uint flags, out long devicePosition, out long qpcPosition);
  int ReleaseBuffer(uint frames);
  int GetNextPacketSize(out uint frames);
}

[StructLayout(LayoutKind.Sequential, Pack = 2)]
public struct WaveFormatEx {
  public short wFormatTag; public short nChannels; public int nSamplesPerSec;
  public int nAvgBytesPerSec; public short nBlockAlign; public short wBitsPerSample; public short cbSize;
}

public static class MicMeter {
  public static double Run(string match, int seconds) {
    var enumerator = (IMMDeviceEnumerator)new MMDeviceEnumeratorComObject();
    IMMDeviceCollection coll;
    int hr = enumerator.EnumAudioEndpoints(1, 0x1, out coll);   // eCapture, ACTIVE
    if (hr != 0) { Console.WriteLine("EnumAudioEndpoints hr=0x" + hr.ToString("X8")); return -1; }

    uint count; coll.GetCount(out count);
    var nameKey = new PropertyKey();
    nameKey.fmtid = new Guid("a45c254e-df1c-4efd-8020-67d146a850e0");
    nameKey.pid = 14;

    IMMDevice target = null; string targetName = null;
    Console.WriteLine("Aktive optageenheder:");
    for (uint i = 0; i < count; i++) {
      IMMDevice dev; coll.Item(i, out dev);
      IPropertyStore store; dev.OpenPropertyStore(0, out store);
      PropVariant pv; store.GetValue(ref nameKey, out pv);
      string name = Marshal.PtrToStringUni(pv.p);
      Console.WriteLine("  - " + name);
      if (target == null && name != null && name.IndexOf(match, StringComparison.OrdinalIgnoreCase) >= 0) {
        target = dev; targetName = name;
      }
    }
    if (target == null) { Console.WriteLine("\nFANDT INGEN enhed der matcher '" + match + "'."); return -1; }
    Console.WriteLine("\nMaaler paa: " + targetName);

    var iidClient = new Guid("1CB9AD4C-DBFA-4C32-B178-C2F568A703B2");
    object o; hr = target.Activate(ref iidClient, 0, IntPtr.Zero, out o);
    Console.WriteLine("  Activate      hr=0x" + hr.ToString("X8"));
    if (hr != 0) return -1;
    var client = (IAudioClient)o;

    IntPtr fmtPtr; hr = client.GetMixFormat(out fmtPtr);
    var fmt = (WaveFormatEx)Marshal.PtrToStructure(fmtPtr, typeof(WaveFormatEx));
    Console.WriteLine(string.Format("  GetMixFormat  hr=0x{0:X8}  -> {1} kanal(er), {2} Hz, {3}-bit, tag {4}",
      hr, fmt.nChannels, fmt.nSamplesPerSec, fmt.wBitsPerSample, fmt.wFormatTag));

    hr = client.Initialize(0, 0, 10000000L, 0, fmtPtr, IntPtr.Zero);   // shared mode, 1 s buffer
    Console.WriteLine("  Initialize    hr=0x" + hr.ToString("X8"));
    if (hr != 0) { Console.WriteLine("  Kunne ikke aabne enheden."); return -1; }

    var iidCapture = new Guid("C8ADBD64-E71E-48A0-A4DE-185C395CD317");
    object c2; hr = client.GetService(ref iidCapture, out c2);
    Console.WriteLine("  GetService    hr=0x" + hr.ToString("X8"));
    if (hr != 0) return -1;
    var capture = (IAudioCaptureClient)c2;

    client.Start();
    Console.WriteLine();
    Console.WriteLine("TAL I HEADSETTET NU - maaler i " + seconds + " sekunder.");
    Console.WriteLine();

    bool isFloat = (fmt.wBitsPerSample == 32);
    double overall = 0.0; long totalFrames = 0;
    var floatBuf = new float[1 << 16];
    var shortBuf = new short[1 << 16];

    for (int sec = 1; sec <= seconds; sec++) {
      double secPeak = 0.0; long secFrames = 0;
      var sw = Stopwatch.StartNew();
      while (sw.ElapsedMilliseconds < 1000) {
        uint packet; capture.GetNextPacketSize(out packet);
        while (packet > 0) {
          IntPtr data; uint frames, flags; long dpos, qpos;
          capture.GetBuffer(out data, out frames, out flags, out dpos, out qpos);
          bool silentFlag = (flags & 0x1) != 0;
          if (frames > 0 && data != IntPtr.Zero && !silentFlag) {
            int n = (int)frames * fmt.nChannels;
            if (isFloat) {
              if (n > floatBuf.Length) n = floatBuf.Length;
              Marshal.Copy(data, floatBuf, 0, n);
              for (int k = 0; k < n; k++) { double a = Math.Abs(floatBuf[k]); if (a > secPeak) secPeak = a; }
            } else {
              if (n > shortBuf.Length) n = shortBuf.Length;
              Marshal.Copy(data, shortBuf, 0, n);
              for (int k = 0; k < n; k++) { double a = Math.Abs((double)shortBuf[k]) / 32768.0; if (a > secPeak) secPeak = a; }
            }
          }
          secFrames += frames;
          capture.ReleaseBuffer(frames);
          capture.GetNextPacketSize(out packet);
        }
        System.Threading.Thread.Sleep(5);
      }
      totalFrames += secFrames;
      if (secPeak > overall) overall = secPeak;
      int bars = (int)(Math.Min(1.0, secPeak) * 50);
      string meter = new string('#', bars).PadRight(50, '.');
      Console.WriteLine(string.Format("  {0,2}s  peak {1:F8}  [{2}]  {3} frames", sec, secPeak, meter, secFrames));
    }
    client.Stop();
    Console.WriteLine();
    Console.WriteLine(string.Format("SAMLET PEAK: {0:F8} over {1} frames", overall, totalFrames));
    return overall;
  }
}
'@

$peak = [MicMeter]::Run($Match, $Seconds)

Write-Host ""
if ($peak -lt 0) {
  Write-Host "MAALINGEN KUNNE IKKE GENNEMFOERES - se hr-koderne ovenfor." -ForegroundColor Red
} elseif ($peak -eq 0.0) {
  Write-Host "EKSAKT DIGITAL STILHED - der loeber ingen lyd i endpointet." -ForegroundColor Red
  Write-Host "  Roeret kan aabnes, men Air Link fylder det ikke. Skift transport (Steam Link) eller mikrofonvej." -ForegroundColor Red
} elseif ($peak -lt 0.01) {
  Write-Host "Der ER lyd, men MEGET svag (peak under 0,01)." -ForegroundColor Yellow
  Write-Host "  Skru Niveauer til 80-100: mmsys.cpl -> Optagelse -> Egenskaber -> Niveauer. Maal igen." -ForegroundColor Yellow
} else {
  Write-Host "LYD BEKRAEFTET - stemmevejen er aaben." -ForegroundColor Green
  Write-Host "  Naeste trin: saet enheden som standard OG standardkommunikationsenhed, saa test-m4.ps1." -ForegroundColor Green
}
