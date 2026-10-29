import { router } from '@kit.ArkUI';
import { Api, ApiError, DeviceItem } from '../common/Api';

interface DeviceParams {
  id: string;
  name: string;
}

@Entry
@Component
struct Devices {
  @State devices: DeviceItem[] = [];
  @State loading: boolean = true;
  @State error: string = '';
  @State unauthorized: boolean = false;

  aboutToAppear(): void {
    this.load();
  }

  async load(): Promise<void> {
    this.loading = true;
    this.error = '';
    this.unauthorized = false;
    try {
      this.devices = await Api.devices();
    } catch (e) {
      const err = e as ApiError;
      this.unauthorized = err.status === 401;
      this.error = Api.errorText(err);
    } finally {
      this.loading = false;
    }
  }

  open(d: DeviceItem): void {
    const params: DeviceParams = { id: d.id, name: d.name };
    router.pushUrl({ url: 'pages/DeviceControl', params: params });
  }

  build() {
    Column() {
      Row() {
        Text($r('app.string.device_my_devices'))
          .fontSize(20)
          .fontWeight(FontWeight.Bold)
        Blank()
        Button($r('app.string.common_back'))
          .fontSize(14)
          .height(32)
          .onClick(() => { router.back(); })
      }
      .width('100%')
      .padding({ left: 16, right: 16, top: 12, bottom: 8 })

      if (this.loading) {
        Column() {
          LoadingProgress().width(40).height(40)
        }
        .width('100%')
        .layoutWeight(1)
        .justifyContent(FlexAlign.Center)
      } else if (this.error !== '') {
        Column({ space: 12 }) {
          Text(this.error)
            .fontSize(14)
            .fontColor('#D32F2F')
          if (this.unauthorized) {
            Button($r('app.string.login_btn'))
              .onClick(() => { router.pushUrl({ url: 'pages/Login' }); })
          }
          Button($r('app.string.common_retry'))
            .backgroundColor('#FFFFFF')
            .fontColor('#333333')
            .onClick(() => { this.load(); })
        }
        .width('100%')
        .layoutWeight(1)
        .justifyContent(FlexAlign.Center)
      } else if (this.devices.length === 0) {
        Column() {
          Text($r('app.string.common_empty'))
            .fontSize(14)
            .fontColor('#888888')
        }
        .width('100%')
        .layoutWeight(1)
        .justifyContent(FlexAlign.Center)
      } else {
        List({ space: 8 }) {
          ForEach(this.devices, (d: DeviceItem) => {
            ListItem() {
              Row({ space: 12 }) {
                Text(d.status === 'online' || d.status === 'enabled' ? '●' : '○')
                  .fontSize(18)
                  .fontColor(d.status === 'online' || d.status === 'enabled' ? '#4CAF50' : '#9E9E9E')
                Column({ space: 4 }) {
                  Text(d.name !== '' ? d.name : d.id).fontSize(16)
                  Text(d.vendor + ' · ' + (d.status === 'online' || d.status === 'enabled'
                    ? $r('app.string.device_online').toString()
                    : $r('app.string.device_offline').toString()))
                    .fontSize(12)
                    .fontColor('#888888')
                }
                .alignItems(HorizontalAlign.Start)
                Blank()
                Button($r('app.string.device_detail'))
                  .fontSize(12)
                  .height(30)
                  .backgroundColor('#FFFFFF')
                  .fontColor($r('app.color.app_primary'))
                  .onClick(() => { this.open(d); })
              }
              .width('100%')
              .padding(12)
              .backgroundColor('#FFFFFF')
              .borderRadius(8)
            }
          }, (d: DeviceItem) => d.id)
        }
        .width('100%')
        .layoutWeight(1)
        .padding({ left: 16, right: 16 })
      }
    }
    .width('100%')
    .height('100%')
    .backgroundColor('#F5F5F5')
  }
}
