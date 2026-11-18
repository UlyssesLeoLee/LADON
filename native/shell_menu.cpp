// Windows-only classic Shell context menu host, built and invoked in an isolated LADON child process.
// Validates a single existing shell item; modern IExplorerCommand Explorer host parity
// is NOT guaranteed. Preserve the Windows Explorer fallback for unsupported handlers.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <shellapi.h>
#include <shlobj.h>
#include <shobjidl.h>
#include <strsafe.h>

namespace {
constexpr UINT kFirstShell = 1;
constexpr UINT kLastShell = 0x6FFF;
constexpr UINT kSensitive = 0x7001;
constexpr UINT kHistory = 0x7002;
constexpr UINT kDocumentTask = 0x7003;
constexpr UINT kExplorer = 0x7004;

struct MenuHost {
    IContextMenu2* context2 = nullptr;
    IContextMenu3* context3 = nullptr;
};

LRESULT CALLBACK ContextWindowProc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam) {
    auto* host = reinterpret_cast<MenuHost*>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));
    if (msg == WM_NCCREATE) {
        auto* create = reinterpret_cast<CREATESTRUCTW*>(lparam);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(create->lpCreateParams));
        return TRUE;
    }
    if (host && (msg == WM_INITMENUPOPUP || msg == WM_DRAWITEM ||
                 msg == WM_MEASUREITEM || msg == WM_MENUCHAR)) {
        LRESULT value = 0;
        if (host->context3 && SUCCEEDED(host->context3->HandleMenuMsg2(msg, wparam, lparam, &value))) {
            return value;
        }
        if (host->context2 && SUCCEEDED(host->context2->HandleMenuMsg(msg, wparam, lparam))) {
            return msg == WM_MENUCHAR ? MAKELRESULT(0, MNC_IGNORE) : 0;
        }
    }
    return DefWindowProcW(hwnd, msg, wparam, lparam);
}

HRESULT QueryShellItemMenu(const wchar_t* path, HWND hwnd, HMENU menu,
                           IContextMenu** out, MenuHost* host) {
    PIDLIST_ABSOLUTE absolute = nullptr;
    HRESULT hr = SHParseDisplayName(path, nullptr, &absolute, 0, nullptr);
    if (FAILED(hr)) return hr;
    IShellFolder* parent = nullptr;
    PCUITEMID_CHILD child = nullptr;
    hr = SHBindToParent(absolute, IID_IShellFolder,
                        reinterpret_cast<void**>(&parent), &child);
    if (SUCCEEDED(hr) && parent) {
        hr = parent->GetUIObjectOf(hwnd, 1, &child, IID_IContextMenu, nullptr,
                                   reinterpret_cast<void**>(out));
        parent->Release();
    }
    CoTaskMemFree(absolute);
    if (FAILED(hr) || !*out) return FAILED(hr) ? hr : E_NOINTERFACE;
    (*out)->QueryInterface(IID_IContextMenu3, reinterpret_cast<void**>(&host->context3));
    (*out)->QueryInterface(IID_IContextMenu2, reinterpret_cast<void**>(&host->context2));
    return (*out)->QueryContextMenu(menu, 0, kFirstShell, kLastShell,
                                     CMF_NORMAL | CMF_EXPLORE | CMF_CANRENAME);
}
} // namespace

// Return 0 for executed Shell command, 0x7001..7004 for LADON commands,
// or negative HRESULT/Win32-style failure code to trigger Explorer fallback.
extern "C" __declspec(dllexport) int ladon_show_shell_menu(const wchar_t* path) {
    if (!path || !*path) return -1;
    DWORD attributes = GetFileAttributesW(path);
    if (attributes == INVALID_FILE_ATTRIBUTES) return -2;
    HRESULT com = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    if (FAILED(com)) return -3;
    WNDCLASSW wc{};
    wc.lpfnWndProc = ContextWindowProc;
    wc.hInstance = GetModuleHandleW(nullptr);
    wc.lpszClassName = L"LadonIsolatedShellMenuHost_2026";
    RegisterClassW(&wc);
    MenuHost host;
    HWND hwnd = CreateWindowExW(WS_EX_TOOLWINDOW, wc.lpszClassName, L"LADON Shell Host",
                                WS_POPUP, 0, 0, 1, 1, nullptr, nullptr, wc.hInstance, &host);
    if (!hwnd) { CoUninitialize(); return -4; }
    HMENU menu = CreatePopupMenu();
    if (!menu) { DestroyWindow(hwnd); CoUninitialize(); return -5; }
    IContextMenu* context = nullptr;
    HRESULT hr = QueryShellItemMenu(path, hwnd, menu, &context, &host);
    int outcome = -6;
    if (SUCCEEDED(hr) && context) {
        AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
        AppendMenuW(menu, MF_STRING, kSensitive, L"添加为敏感文件  (.LADONignore)");
        AppendMenuW(menu, MF_STRING, kHistory, L"查看 LADON 完整修改历史");
        AppendMenuW(menu, MF_STRING, kDocumentTask, L"文档即任务");
        AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
        AppendMenuW(menu, MF_STRING, kExplorer, L"在 Windows Explorer 中打开");
        POINT pt{};
        GetCursorPos(&pt);
        SetForegroundWindow(hwnd);
        UINT chosen = TrackPopupMenuEx(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON,
                                       pt.x, pt.y, hwnd, nullptr);
        PostMessageW(hwnd, WM_NULL, 0, 0);
        if (chosen == 0) outcome = 0; // dismissed
        else if (chosen >= kSensitive && chosen <= kExplorer) outcome = static_cast<int>(chosen);
        else if (chosen >= kFirstShell && chosen <= kLastShell) {
            CMINVOKECOMMANDINFOEX command{};
            command.cbSize = sizeof(command);
            command.fMask = CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE;
            command.hwnd = hwnd;
            command.lpVerb = MAKEINTRESOURCEA(chosen - kFirstShell);
            command.lpVerbW = MAKEINTRESOURCEW(chosen - kFirstShell);
            command.nShow = SW_SHOWNORMAL;
            command.ptInvoke = pt;
            outcome = SUCCEEDED(context->InvokeCommand(reinterpret_cast<LPCMINVOKECOMMANDINFO>(&command))) ? 0 : -7;
        }
    }
    if (host.context3) host.context3->Release();
    if (host.context2) host.context2->Release();
    if (context) context->Release();
    DestroyMenu(menu);
    DestroyWindow(hwnd);
    CoUninitialize();
    return outcome;
}
