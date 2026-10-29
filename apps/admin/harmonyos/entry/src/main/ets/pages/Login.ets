import { router } from '@kit.ArkUI';
import { Api, ApiError } from '../common/Api';

// 登录页：用户名/密码 → 网关 /api/auth/login，JWT 由 Api 持久化。
@Entry
@Component
struct Login {
  @State username: string = '';
  @State password: string = '';
  @State error: string = '';
  @State busy: boolean = false;

  async doLogin(): Promise<void> {
    if (this.username === '' || this.password === '') {
      this.error = $r('app.string.login_required').toString();
      return;
    }
    if (this.busy) {
      return;
    }
    this.busy = true;
    this.error = '';
    try {
      await Api.login(this.username, this.password);
      router.back();
    } catch (e) {
      const err = e as ApiError;
      this.error = err.status === 401
        ? $r('app.string.login_error').toString()
        : Api.errorText(err);
    } finally {
      this.busy = false;
    }
  }

  build() {
    Column({ space: 12 }) {
      Text($r('app.string.login_title'))
        .fontSize(24)
        .fontWeight(FontWeight.Bold)
      Text($r('app.string.login_subtitle'))
        .fontSize(14)
        .fontColor('#888888')
      TextInput({ placeholder: $r('app.string.login_username').toString() })
        .onChange((v: string) => { this.username = v; })
        .backgroundColor('#FFFFFF')
        .borderRadius(8)
      TextInput({ placeholder: $r('app.string.login_password').toString() })
        .type(InputType.Password)
        .onChange((v: string) => { this.password = v; })
        .backgroundColor('#FFFFFF')
        .borderRadius(8)
      if (this.error !== '') {
        Text(this.error)
          .fontSize(14)
          .fontColor('#D32F2F')
      }
      Button($r('app.string.login_btn'))
        .width('100%')
        .enabled(!this.busy)
        .onClick(() => { this.doLogin(); })
      Button($r('app.string.common_back'))
        .width('100%')
        .backgroundColor('#FFFFFF')
        .fontColor('#333333')
        .onClick(() => { router.back(); })
    }
    .width('100%')
    .height('100%')
    .padding(24)
    .justifyContent(FlexAlign.Center)
    .backgroundColor('#F5F5F5')
  }
}
