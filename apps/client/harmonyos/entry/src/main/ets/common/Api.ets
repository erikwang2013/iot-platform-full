import { http } from '@kit.NetworkKit';
import { preferences } from '@kit.ArkData';

const PREF_NAME = 'api_store';
const KEY_TOKEN = 'token';

export interface DeviceItem {
  id: string;
  name: string;
  vendor: string;
  status: string;
}

interface DeviceListResp {
  devices?: DeviceItem[];
}

export interface ThingProperty {
  identifier: string;
  name?: string;
  type?: string;
  unit?: string;
  rw?: string;
  min?: number;
  max?: number;
  enum?: string[];
}

export interface ThingModelResp {
  properties?: ThingProperty[];
  events?: ThingProperty[];
  services?: ThingProperty[];
}

interface LoginResp {
  token?: string;
}

interface LoginBody {
  username: string;
  password: string;
}

interface CommandBody {
  code: string;
  value: string | number | boolean;
}

interface CommandResp {
  ok?: boolean;
  queued?: boolean;
  channel?: string;
}

export interface CommandResult {
  ok: boolean;
  queued: boolean;
  channel: string;
}

export class ApiError extends Error {
  status: number;

  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

/// 网关 API：JWT 持久化 + 设备/物模型/指令端点（与 Flutter 端共用同一网关）。
export class Api {
  // shortcut: 网关地址写死本地开发默认值，多环境联调时改为设置页可配置
  static baseUrl: string = 'http://localhost:8080';
  // 管理端登录；客户端 App 的 Api.ets 使用 /admin/auth/login
  static loginPath: string = '/admin/auth/login';

  private static token: string = '';
  private static loaded: boolean = false;
  private static prefs: preferences.Preferences | null = null;

  private static async prefsInstance(): Promise<preferences.Preferences> {
    if (!Api.prefs) {
      Api.prefs = await preferences.getPreferences(getContext(), PREF_NAME);
    }
    return Api.prefs;
  }

  static async init(): Promise<void> {
    if (Api.loaded) {
      return;
    }
    const p = await Api.prefsInstance();
    Api.token = await p.get(KEY_TOKEN, '') as string;
    Api.loaded = true;
  }

  static async login(username: string, password: string): Promise<void> {
    const body: LoginBody = { username: username, password: password };
    const text = await Api.request(http.RequestMethod.POST, Api.loginPath, JSON.stringify(body));
    const resp = JSON.parse(text) as LoginResp;
    if (!resp.token) {
      throw new ApiError(500, 'missing token');
    }
    Api.token = resp.token;
    Api.loaded = true;
    const p = await Api.prefsInstance();
    await p.put(KEY_TOKEN, Api.token);
    await p.flush();
  }

  static async devices(): Promise<DeviceItem[]> {
    const text = await Api.request(http.RequestMethod.GET, '/api/devices');
    const resp = JSON.parse(text) as DeviceListResp;
    return resp.devices ? resp.devices : [];
  }

  static async thingModel(deviceId: string): Promise<ThingModelResp> {
    const text = await Api.request(http.RequestMethod.GET,
      '/api/models/things/' + encodeURIComponent(deviceId));
    return JSON.parse(text) as ThingModelResp;
  }

  static async sendCommand(deviceId: string, code: string,
    value: string | number | boolean): Promise<CommandResult> {
    const body: CommandBody = { code: code, value: value };
    const text = await Api.request(http.RequestMethod.POST,
      '/api/access/devices/' + encodeURIComponent(deviceId) + '/command', JSON.stringify(body));
    const resp = JSON.parse(text) as CommandResp;
    const result: CommandResult = {
      ok: resp.ok === true,
      queued: resp.queued === true,
      channel: resp.channel ? resp.channel : ''
    };
    return result;
  }

  /// 网关错误（401/403/404/5xx/网络）→ 本地化文案。
  static errorText(err: ApiError): string {
    if (err.status === 0) {
      return $r('app.string.error_network_error').toString();
    }
    if (err.status === 401) {
      return $r('app.string.error_unauthorized').toString();
    }
    if (err.status === 403) {
      return $r('app.string.error_forbidden').toString();
    }
    if (err.status === 404) {
      return $r('app.string.error_not_found').toString();
    }
    if (err.status >= 500) {
      return $r('app.string.error_server_error').toString();
    }
    return err.message !== '' ? err.message : $r('app.string.common_error').toString();
  }

  private static async request(method: http.RequestMethod, path: string,
    body?: string): Promise<string> {
    await Api.init();
    const header: Record<string, string> = {
      'Content-Type': 'application/json',
      'X-API-Version': 'v1'
    };
    if (Api.token !== '') {
      header['Authorization'] = 'Bearer ' + Api.token;
    }
    const options: http.HttpRequestOptions = {
      method: method,
      header: header,
      connectTimeout: 10000,
      readTimeout: 10000,
      expectDataType: http.HttpDataType.STRING,
      extraData: body
    };
    const req = http.createHttp();
    let text: string = '';
    let status: number = 0;
    try {
      const resp = await req.request(Api.baseUrl + path, options);
      text = resp.result as string;
      status = resp.responseCode;
    } catch (e) {
      req.destroy();
      throw new ApiError(0, 'network');
    }
    req.destroy();
    if (status === 401) {
      Api.token = '';
      const p = await Api.prefsInstance();
      await p.delete(KEY_TOKEN);
      await p.flush();
      throw new ApiError(401, text);
    }
    if (status >= 400) {
      throw new ApiError(status, text);
    }
    return text;
  }
}
