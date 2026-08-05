# Laeser peak-maaleren paa HVER aktiv afspilningsenhed - samme tal Windows'
# lydindstillinger viser som soejle. Afgoer om Oculus-endpointet rapporterer en
# KONSTANT vaerdi (en fastlaast maaler) eller reelt nul (et tomt roer).
#
# Brug: powershell -ExecutionPolicy Bypass -File measure-render-meters.ps1 [-Seconds 6] [-Beep]

param([int]$Seconds = 6, [switch]$Beep)

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

[ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
public class MtrEnumComObject { }

[ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMtrEnum {
  int EnumAudioEndpoints(int flow, int mask, out IMtrColl c);
  int GetDefaultAudioEndpoint(int flow, int role, out IMtrDev d);
  int GetDevice(string id, out IMtrDev d);
  int RegisterEndpointNotificationCallback(IntPtr c);
  int UnregisterEndpointNotificationCallback(IntPtr c);
}

[ComImport, Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMtrColl { int GetCount(out uint c); int Item(uint i, out IMtrDev d); }

[ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMtrDev {
  int Activate(ref Guid iid, uint ctx, IntPtr p, [MarshalAs(UnmanagedType.IUnknown)] out object o);
  int OpenPropertyStore(uint a, out IMtrStore s);
  int GetId([MarshalAs(UnmanagedType.LPWStr)] out string id);
  int GetState(out uint st);
}

[StructLayout(LayoutKind.Sequential)] public struct MtrKey { public Guid fmtid; public int pid; }
[StructLayout(LayoutKind.Explicit)] public struct MtrVar { [FieldOffset(0)] public short vt; [FieldOffset(8)] public IntPtr p; }

[ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IMtrStore {
  int GetCount(out uint c); int GetAt(uint i, out MtrKey k);
  int GetValue(ref MtrKey k, out MtrVar v); int SetValue(ref MtrKey k, ref MtrVar v); int Commit();
}

// IAudioMeterInformation - praecis den kilde Windows' lydside bruger til soejlen
[ComImport, Guid("C02216F6-8C67-4B5B-9D00-D008E73E0064"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IAudioMeter {
  int GetPeakValue(out float peak);
  int GetMeteringChannelCount(out uint count);
  int GetChannelsPeakValues(uint count, [Out] float[] peaks);
  int QueryHardwareSupport(out uint mask);
}

// IAudioEndpointVolume - for at se om enheden er mutet eller staar lavt
[ComImport, Guid("5CDF2C82-841E-4546-9722-0CF74078229A"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IEndpointVolume {
  int RegisterControlChangeNotify(IntPtr n);
  int UnregisterControlChangeNotify(IntPtr n);
  int GetChannelCount(out uint c);
  int SetMasterVolumeLevel(float db, ref Guid ctx);
  int SetMasterVolumeLevelScalar(float lvl, ref Guid ctx);
  int GetMasterVolumeLevel(out float db);
  int GetMasterVolumeLevelScalar(out float lvl);
  int SetChannelVolumeLevel(uint ch, float db, ref Guid ctx);
  int SetChannelVolumeLevelScalar(uint ch, float lvl, ref Guid ctx);
  int GetChannelVolumeLevel(uint ch, out float db);
  int GetChannelVolumeLevelScalar(uint ch, out float lvl);
  int SetMute(int mute, ref Guid ctx);
  int GetMute(out int mute);
}

public static class RenderMeters {
  class Row { public string Name; public IAudioMeter Meter; public List<float> Samples = new List<float>();
              public float Vol; public bool Mute; }

  static string NameOf(IMtrDev d) {
    IMtrStore s; d.OpenPropertyStore(0, out s);
    var k = new MtrKey(); k.fmtid = new Guid("a45c254e-df1c-4efd-8020-67d146a850e0"); k.pid = 14;
    MtrVar v; s.GetValue(ref k, out v);
    return Marshal.PtrToStringUni(v.p);
  }

  public static void Run(int seconds) {
    var e = (IMtrEnum)new MtrEnumComObject();
    IMtrColl coll; e.EnumAudioEndpoints(0, 0x1, out coll);   // eRender, ACTIVE
    uint n; coll.GetCount(out n);

    var iidMeter = new Guid("C02216F6-8C67-4B5B-9D00-D008E73E0064");
    var iidVol = new Guid("5CDF2C82-841E-4546-9722-0CF74078229A");
    var rows = new List<Row>();

    for (uint i = 0; i < n; i++) {
      IMtrDev d; coll.Item(i, out d);
      object mo;
      if (d.Activate(ref iidMeter, 0, IntPtr.Zero, out mo) != 0) continue;
      var row = new Row { Name = NameOf(d), Meter = (IAudioMeter)mo };
      object vo;
      if (d.Activate(ref iidVol, 0, IntPtr.Zero, out vo) == 0) {
        var ev = (IEndpointVolume)vo;
        float lvl; ev.GetMasterVolumeLevelScalar(out lvl); row.Vol = lvl;
        int m; ev.GetMute(out m); row.Mute = (m != 0);
      }
      rows.Add(row);
    }

    Console.WriteLine("Maaler peak paa " + rows.Count + " aktive afspilningsenheder i " + seconds + " sekunder.");
    Console.WriteLine("Spil noget lyd imens - fx en video eller en systemlyd.");
    Console.WriteLine();

    int ticks = seconds * 10;
    for (int t = 0; t < ticks; t++) {
      foreach (var r in rows) {
        float p; if (r.Meter.GetPeakValue(out p) == 0) r.Samples.Add(p);
      }
      System.Threading.Thread.Sleep(100);
    }

    Console.WriteLine("=== RESULTAT ===");
    foreach (var r in rows) {
      float mn = 1f, mx = 0f; double sum = 0;
      foreach (var s in r.Samples) { if (s < mn) mn = s; if (s > mx) mx = s; sum += s; }
      float avg = r.Samples.Count > 0 ? (float)(sum / r.Samples.Count) : 0f;
      string diag;
      if (r.Samples.Count == 0) diag = "ingen maalinger";
      else if (mx == 0f) diag = "HELT TAVS (peak 0 hele vejen)";
      else if (mx - mn < 0.0001f) diag = "FASTLAAST paa " + mx.ToString("F4") + " - maaleren bevaeger sig IKKE";
      else diag = "levende signal";
      Console.WriteLine(string.Format("  {0,-52} min {1:F4} max {2:F4} snit {3:F4}  vol {4:P0}{5}",
        r.Name, mn, mx, avg, r.Vol, r.Mute ? " MUTET" : ""));
      Console.WriteLine("      -> " + diag);
    }
    Console.WriteLine();
    Console.WriteLine("En FASTLAAST maaler betyder at driveren rapporterer en konstant vaerdi");
    Console.WriteLine("i stedet for faktisk lyd - roeret er ikke forbundet til noget der spiller.");
  }
}
'@

if ($Beep) {
  Start-Job -ScriptBlock {
    Add-Type -AssemblyName System.Windows.Forms
    for ($i = 0; $i -lt 12; $i++) { [System.Media.SystemSounds]::Asterisk.Play(); Start-Sleep -Milliseconds 450 }
  } | Out-Null
  "Spiller systemlyde i baggrunden imens..."
}

[RenderMeters]::Run($Seconds)
