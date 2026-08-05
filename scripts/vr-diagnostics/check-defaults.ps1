# Viser hvilken enhed Windows leverer for HVER rolle, ind og ud.
# Chromium/WebView2 slaar op paa de reserverede id'er 'default' (eConsole) og
# 'communications' (eCommunications). De kan pege paa hver sin enhed, og
# Talminal har ingen deviceId-constraint, saa begge SKAL vaere rigtige.

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

[ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
public class DefEnumComObject { }

[ComImport, Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IDefEnum {
  int EnumAudioEndpoints(int dataFlow, int stateMask, out IntPtr devices);
  int GetDefaultAudioEndpoint(int dataFlow, int role, out IDefDevice device);
  int GetDevice(string id, out IDefDevice device);
  int RegisterEndpointNotificationCallback(IntPtr client);
  int UnregisterEndpointNotificationCallback(IntPtr client);
}

[ComImport, Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IDefDevice {
  int Activate(ref Guid iid, uint clsCtx, IntPtr activationParams, [MarshalAs(UnmanagedType.IUnknown)] out object iface);
  int OpenPropertyStore(uint access, out IDefStore store);
  int GetId([MarshalAs(UnmanagedType.LPWStr)] out string id);
  int GetState(out uint state);
}

[StructLayout(LayoutKind.Sequential)]
public struct DefKey { public Guid fmtid; public int pid; }

[StructLayout(LayoutKind.Explicit)]
public struct DefVariant { [FieldOffset(0)] public short vt; [FieldOffset(8)] public IntPtr p; }

[ComImport, Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IDefStore {
  int GetCount(out uint count);
  int GetAt(uint index, out DefKey key);
  int GetValue(ref DefKey key, out DefVariant value);
  int SetValue(ref DefKey key, ref DefVariant value);
  int Commit();
}

public static class DefaultAudio {
  static string NameOf(IDefDevice dev) {
    IDefStore store; dev.OpenPropertyStore(0, out store);
    var key = new DefKey();
    key.fmtid = new Guid("a45c254e-df1c-4efd-8020-67d146a850e0");
    key.pid = 14;
    DefVariant pv; store.GetValue(ref key, out pv);
    return Marshal.PtrToStringUni(pv.p);
  }

  public static void Report() {
    var e = (IDefEnum)new DefEnumComObject();
    string[] flows = { "IND  (optagelse)", "UD   (afspilning)" };
    string[] roles = { "default        (eConsole)", "multimedia     (eMultimedia)", "communications (eCommunications)" };
    for (int flow = 1; flow >= 0; flow--) {
      Console.WriteLine(flows[flow == 1 ? 0 : 1]);
      for (int role = 0; role < 3; role++) {
        IDefDevice dev;
        int hr = e.GetDefaultAudioEndpoint(flow == 1 ? 1 : 0, role, out dev);
        if (hr != 0) { Console.WriteLine("  " + roles[role] + " -> hr=0x" + hr.ToString("X8")); continue; }
        Console.WriteLine("  " + roles[role] + " -> " + NameOf(dev));
      }
      Console.WriteLine();
    }
  }
}
'@

[DefaultAudio]::Report()
