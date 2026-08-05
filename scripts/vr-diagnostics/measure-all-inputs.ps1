# Aabner ALLE aktive optageenheder samtidig og maaler peak paa hver.
# Du taler EEN gang - rapporten viser hvilken enhed der faktisk hoerte dig.
# WASAPI shared mode tillader flere klienter, saa parallelle streams er lovligt.
#
# Brug: powershell -ExecutionPolicy Bypass -File measure-all-inputs.ps1 [-Seconds 15]

param([int]$Seconds = 15)

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;

[ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
public class AllEnumComObject { }

[ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IAllEnum {
  int EnumAudioEndpoints(int dataFlow, int stateMask, out IAllColl devices);
  int GetDefaultAudioEndpoint(int dataFlow, int role, out IAllDev device);
  int GetDevice(string id, out IAllDev device);
  int RegisterEndpointNotificationCallback(IntPtr c);
  int UnregisterEndpointNotificationCallback(IntPtr c);
}

[ComImport, Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IAllColl { int GetCount(out uint c); int Item(uint i, out IAllDev d); }

[ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IAllDev {
  int Activate(ref Guid iid, uint ctx, IntPtr p, [MarshalAs(UnmanagedType.IUnknown)] out object o);
  int OpenPropertyStore(uint access, out IAllStore s);
  int GetId([MarshalAs(UnmanagedType.LPWStr)] out string id);
  int GetState(out uint state);
}

[StructLayout(LayoutKind.Sequential)] public struct AllKey { public Guid fmtid; public int pid; }
[StructLayout(LayoutKind.Explicit)] public struct AllVar { [FieldOffset(0)] public short vt; [FieldOffset(8)] public IntPtr p; }

[ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IAllStore {
  int GetCount(out uint c); int GetAt(uint i, out AllKey k);
  int GetValue(ref AllKey k, out AllVar v); int SetValue(ref AllKey k, ref AllVar v); int Commit();
}

[ComImport, Guid("1CB9AD4C-DBFA-4C32-B178-C2F568A703B2"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IAllClient {
  int Initialize(int mode, int flags, long dur, long per, IntPtr fmt, IntPtr guid);
  int GetBufferSize(out uint f); int GetStreamLatency(out long l); int GetCurrentPadding(out uint p);
  int IsFormatSupported(int mode, IntPtr fmt, IntPtr closest);
  int GetMixFormat(out IntPtr fmt); int GetDevicePeriod(out long d, out long m);
  int Start(); int Stop(); int Reset(); int SetEventHandle(IntPtr h);
  int GetService(ref Guid iid, [MarshalAs(UnmanagedType.IUnknown)] out object o);
}

[ComImport, Guid("C8ADBD64-E71E-48A0-A4DE-185C395CD317"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IAllCapture {
  int GetBuffer(out IntPtr data, out uint frames, out uint flags, out long dpos, out long qpos);
  int ReleaseBuffer(uint frames);
  int GetNextPacketSize(out uint frames);
}

[StructLayout(LayoutKind.Sequential, Pack = 2)]
public struct AllFmt {
  public short tag; public short ch; public int rate; public int avg;
  public short align; public short bits; public short cb;
}

public static class AllMic {
  class Dev {
    public string Name; public IAllClient Client; public IAllCapture Cap;
    public int Ch; public bool Float; public double Peak; public long Frames;
  }

  static string NameOf(IAllDev d) {
    IAllStore s; d.OpenPropertyStore(0, out s);
    var k = new AllKey(); k.fmtid = new Guid("a45c254e-df1c-4efd-8020-67d146a850e0"); k.pid = 14;
    AllVar v; s.GetValue(ref k, out v);
    return Marshal.PtrToStringUni(v.p);
  }

  public static void Run(int seconds) {
    var e = (IAllEnum)new AllEnumComObject();

    // Foerst: vis ALLE endpoints inkl. dem der ikke er aktive - det er selve pointen.
    IAllColl all;
    e.EnumAudioEndpoints(1, 0x0F, out all);   // eCapture, ALL states
    uint total; all.GetCount(out total);
    Console.WriteLine("Optageenheder paa maskinen:");
    var live = new List<IAllDev>();
    var liveNames = new List<string>();
    for (uint i = 0; i < total; i++) {
      IAllDev d; all.Item(i, out d);
      uint st; d.GetState(out st);
      string word = st == 1 ? "AKTIV     " : st == 2 ? "DISABLED  " : st == 4 ? "NOTPRESENT" : st == 8 ? "UNPLUGGED " : ("state" + st);
      string nm = NameOf(d);
      Console.WriteLine("  [" + word + "] " + nm);
      if (st == 1) { live.Add(d); liveNames.Add(nm); }
    }

    var devs = new List<Dev>();
    var iidClient = new Guid("1CB9AD4C-DBFA-4C32-B178-C2F568A703B2");
    var iidCap = new Guid("C8ADBD64-E71E-48A0-A4DE-185C395CD317");

    Console.WriteLine();
    for (int i = 0; i < live.Count; i++) {
      object o;
      int hr = live[i].Activate(ref iidClient, 0, IntPtr.Zero, out o);
      if (hr != 0) { Console.WriteLine("  kunne ikke aabne: " + liveNames[i] + " hr=0x" + hr.ToString("X8")); continue; }
      var cl = (IAllClient)o;
      IntPtr fp; cl.GetMixFormat(out fp);
      var fmt = (AllFmt)Marshal.PtrToStructure(fp, typeof(AllFmt));
      hr = cl.Initialize(0, 0, 10000000L, 0, fp, IntPtr.Zero);
      if (hr != 0) { Console.WriteLine("  Initialize fejlede: " + liveNames[i] + " hr=0x" + hr.ToString("X8")); continue; }
      object c2; hr = cl.GetService(ref iidCap, out c2);
      if (hr != 0) { Console.WriteLine("  GetService fejlede: " + liveNames[i]); continue; }
      cl.Start();
      devs.Add(new Dev { Name = liveNames[i], Client = cl, Cap = (IAllCapture)c2,
                         Ch = fmt.ch, Float = (fmt.bits == 32), Peak = 0.0, Frames = 0 });
      Console.WriteLine("  lytter paa: " + liveNames[i] + "  (" + fmt.ch + "ch " + fmt.rate + "Hz " + fmt.bits + "bit)");
    }

    if (devs.Count == 0) { Console.WriteLine("Ingen enheder kunne aabnes."); return; }

    Console.WriteLine();
    for (int c = 3; c >= 1; c--) { Console.WriteLine("  ... " + c); System.Threading.Thread.Sleep(700); }
    Console.WriteLine();
    Console.WriteLine("TAL NU - tal normalt i " + seconds + " sekunder, gerne et par saetninger.");
    Console.WriteLine();

    var fbuf = new float[1 << 16];
    var sbuf = new short[1 << 16];
    var sw = Stopwatch.StartNew();
    long lastReport = 0;

    while (sw.ElapsedMilliseconds < seconds * 1000L) {
      foreach (var d in devs) {
        uint packet; d.Cap.GetNextPacketSize(out packet);
        while (packet > 0) {
          IntPtr data; uint frames, flags; long dp, qp;
          d.Cap.GetBuffer(out data, out frames, out flags, out dp, out qp);
          if (frames > 0 && data != IntPtr.Zero && (flags & 0x1) == 0) {
            int n = (int)frames * d.Ch;
            if (d.Float) {
              if (n > fbuf.Length) n = fbuf.Length;
              Marshal.Copy(data, fbuf, 0, n);
              for (int k = 0; k < n; k++) { double a = Math.Abs(fbuf[k]); if (a > d.Peak) d.Peak = a; }
            } else {
              if (n > sbuf.Length) n = sbuf.Length;
              Marshal.Copy(data, sbuf, 0, n);
              for (int k = 0; k < n; k++) { double a = Math.Abs((double)sbuf[k]) / 32768.0; if (a > d.Peak) d.Peak = a; }
            }
          }
          d.Frames += frames;
          d.Cap.ReleaseBuffer(frames);
          d.Cap.GetNextPacketSize(out packet);
        }
      }
      if (sw.ElapsedMilliseconds - lastReport >= 1000) {
        lastReport = sw.ElapsedMilliseconds;
        Console.Write("  " + (lastReport / 1000) + "s ");
        foreach (var d in devs) {
          int bars = (int)(Math.Min(1.0, d.Peak) * 14);
          Console.Write(" | " + new string('#', bars).PadRight(14, '.'));
        }
        Console.WriteLine();
      }
      System.Threading.Thread.Sleep(4);
    }

    foreach (var d in devs) d.Client.Stop();

    Console.WriteLine();
    Console.WriteLine("=== RESULTAT (peak over hele maalingen) ===");
    Dev best = null;
    foreach (var d in devs) {
      string verdict = d.Peak >= 0.05 ? "HOERTE DIG" : d.Peak > 0.005 ? "svagt signal" : d.Peak > 0.0 ? "kun stoejgulv" : "EKSAKT STILHED";
      Console.WriteLine(string.Format("  {0,-52} peak {1:F8}  {2}  ({3} frames)", d.Name, d.Peak, verdict, d.Frames));
      if (best == null || d.Peak > best.Peak) best = d;
    }
    Console.WriteLine();
    if (best != null && best.Peak >= 0.05) {
      Console.WriteLine("Enheden der hoerte dig: " + best.Name);
      Console.WriteLine("Saet PRAECIS den som baade standardenhed OG standardkommunikationsenhed,");
      Console.WriteLine("saa bruger Talminal den (appen har ingen egen enhedsvaelger endnu).");
    } else {
      Console.WriteLine("INGEN enhed hoerte dig tydeligt. Enten talte du ikke, eller headsettets");
      Console.WriteLine("mikrofon naar slet ikke Windows i denne transport.");
    }
  }
}
'@

[AllMic]::Run($Seconds)
