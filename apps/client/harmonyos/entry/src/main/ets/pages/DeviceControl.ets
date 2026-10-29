import { router } from '@kit.ArkUI';
import { Api, ApiError, CommandResult, ThingModelResp, ThingProperty } from '../common/Api';

interface DeviceParams {
  id: string;
  name: string;
}

/// 设备详情/控制：物模型动态渲染属性控制，指令经 /api/access/devices/{id}/command 下发。
@Entry
@Component
struct DeviceControl {
  @State deviceId: string = '';
  @State deviceName: string = '';
  @State properties: ThingProperty[] = [];
  @State events: ThingProperty[] = [];
  @State services: ThingProperty[] = [];
  @State loading: boolean = true;
  @State busy: boolean = false;
  @State error: string = '';
  @State result: string = '';
  @State switches: Map<string, boolean> = new Map<string, boolean>();
  @State enumIndexes: Map<string, number> = new Map<string, number>();
  private inputs: Map<string, string> = new Map<string, string>();

  aboutToAppear(): void {
    const raw = router.getParams();
    if (raw !== undefined && raw !== null) {
      const params = raw as DeviceParams;
      this.deviceId = params.id;
      this.deviceName = params.name;
    }
    this.load();
  }

  async load(): Promise<void> {
    this.loading = true;
    this.error = '';
    this.result = '';
    try {
      const model: ThingModelResp = await Api.thingModel(this.deviceId);
      this.properties = model.properties ? model.properties : [];
      this.events = model.events ? model.events : [];
      this.services = model.services ? model.services : [];
    } catch (e) {
      this.error = Api.errorText(e as ApiError);
    } finally {
      this.loading = false;
    }
  }

  async send(code: string, value: string | number | boolean): Promise<void> {
    if (this.busy) {
      return;
    }
    this.busy = true;
    this.error = '';
    this.result = '';
    try {
      const r: CommandResult = await Api.sendCommand(this.deviceId, code, value);
      const state = r.queued ? 'queued' : (r.ok ? 'ok' : 'failed');
      this.result = $r('app.string.command_sent').toString() + ' · ' + state
        + (r.channel !== '' ? ' · ' + r.channel : '');
    } catch (e) {
      this.error = Api.errorText(e as ApiError);
    } finally {
      this.busy = false;
    }
  }

  sendText(code: string, numeric: boolean): void {
    const text = this.inputs.get(code);
    const value = text !== undefined ? text : '';
    if (numeric) {
      const n = Number(value);
      if (value === '' || Number.isNaN(n)) {
        this.error = $r('app.string.common_error').toString();
        return;
      }
      this.send(code, n);
      return;
    }
    this.send(code, value);
  }

  sendEnum(code: string, values: string[] | undefined, index: number): void {
    if (!values || index < 0 || index >= values.length) {
      return;
    }
    this.enumIndexes.set(code, index);
    this.send(code, values[index]);
  }

  enumIndex(code: string): number {
    const v = this.enumIndexes.get(code);
    return v !== undefined ? v : 0;
  }

  enumLabel(p: ThingProperty): string {
    const values = p.enum ? p.enum : [];
    const i = this.enumIndex(p.identifier);
    return values.length > 0 && i < values.length ? values[i] : '';
  }

  toggle(code: string, on: boolean): void {
    this.switches.set(code, on);
    this.send(code, on);
  }

  switchValue(code: string): boolean {
    const v = this.switches.get(code);
    return v !== undefined ? v : false;
  }

  propertyLabel(p: ThingProperty): string {
    return p.name !== undefined && p.name !== '' ? p.name : p.identifier;
  }

  propertyType(p: ThingProperty): string {
    return p.type !== undefined && p.type !== '' ? p.type : 'string';
  }

  propertyUnit(p: ThingProperty): string {
    return p.unit !== undefined && p.unit !== '' ? ' · ' + p.unit : '';
  }

  writable(p: ThingProperty): boolean {
    return p.rw !== 'r';
  }

  enumOptions(p: ThingProperty): SelectOption[] {
    const out: SelectOption[] = [];
    const values = p.enum ? p.enum : [];
    for (const v of values) {
      out.push({ value: v });
    }
    return out;
  }

  build() {
    Column() {
      Row() {
        Column({ space: 2 }) {
          Text(this.deviceName !== '' ? this.deviceName : this.deviceId)
            .fontSize(20)
            .fontWeight(FontWeight.Bold)
          Text($r('app.string.device_control'))
            .fontSize(12)
            .fontColor('#888888')
        }
        .alignItems(HorizontalAlign.Start)
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
      } else if (this.error !== '' && this.properties.length === 0
        && this.events.length === 0 && this.services.length === 0) {
        Column({ space: 12 }) {
          Text(this.error)
            .fontSize(14)
            .fontColor('#D32F2F')
          Button($r('app.string.common_retry'))
            .onClick(() => { this.load(); })
        }
        .width('100%')
        .layoutWeight(1)
        .justifyContent(FlexAlign.Center)
      } else {
        Scroll() {
          Column({ space: 8 }) {
            if (this.properties.length === 0 && this.events.length === 0
              && this.services.length === 0) {
              Text($r('app.string.common_empty'))
                .fontSize(14)
                .fontColor('#888888')
                .padding(24)
            }
            if (this.properties.length > 0) {
              Text($r('app.string.model_property'))
                .fontSize(14)
                .fontColor('#999999')
                .width('100%')
              ForEach(this.properties, (p: ThingProperty) => {
                Column({ space: 8 }) {
                  Row() {
                    Text(this.propertyLabel(p))
                      .fontSize(16)
                    Blank()
                    Text(this.propertyType(p) + this.propertyUnit(p))
                      .fontSize(12)
                      .fontColor('#888888')
                  }
                  .width('100%')
                  if (this.writable(p)) {
                    if (this.propertyType(p) === 'bool') {
                      Toggle({ type: ToggleType.Switch, isOn: this.switchValue(p.identifier) })
                        .onChange((on: boolean) => { this.toggle(p.identifier, on); })
                    } else if (this.propertyType(p) === 'number') {
                      Row({ space: 8 }) {
                        TextInput({ placeholder: p.identifier })
                          .type(InputType.NUMBER_DECIMAL)
                          .layoutWeight(1)
                          .onChange((v: string) => { this.inputs.set(p.identifier, v); })
                        Button('→')
                          .height(36)
                          .enabled(!this.busy)
                          .onClick(() => { this.sendText(p.identifier, true); })
                      }
                      .width('100%')
                    } else if (this.propertyType(p) === 'enum'
                      && p.enum !== undefined && p.enum.length > 0) {
                      Select(this.enumOptions(p))
                        .selected(this.enumIndex(p.identifier))
                        .value(this.enumLabel(p))
                        .onSelect((index: number) => {
                          this.sendEnum(p.identifier, p.enum, index);
                        })
                    } else {
                      Row({ space: 8 }) {
                        TextInput({ placeholder: p.identifier })
                          .layoutWeight(1)
                          .onChange((v: string) => { this.inputs.set(p.identifier, v); })
                        Button('→')
                          .height(36)
                          .enabled(!this.busy)
                          .onClick(() => { this.sendText(p.identifier, false); })
                      }
                      .width('100%')
                    }
                  } else {
                    Text('—')
                      .fontSize(14)
                      .fontColor('#888888')
                  }
                }
                .width('100%')
                .padding(12)
                .backgroundColor('#FFFFFF')
                .borderRadius(8)
                .alignItems(HorizontalAlign.Start)
              }, (p: ThingProperty) => p.identifier)
            }
            if (this.events.length > 0) {
              Text($r('app.string.model_event'))
                .fontSize(14)
                .fontColor('#999999')
                .width('100%')
              ForEach(this.events, (e: ThingProperty) => {
                Text(this.propertyLabel(e) + ' · event')
                  .fontSize(14)
                  .width('100%')
                  .padding(12)
                  .backgroundColor('#FFFFFF')
                  .borderRadius(8)
              }, (e: ThingProperty) => e.identifier)
            }
            if (this.services.length > 0) {
              Text($r('app.string.model_service'))
                .fontSize(14)
                .fontColor('#999999')
                .width('100%')
              ForEach(this.services, (s: ThingProperty) => {
                Text(this.propertyLabel(s) + ' · ' + s.identifier)
                  .fontSize(14)
                  .width('100%')
                  .padding(12)
                  .backgroundColor('#FFFFFF')
                  .borderRadius(8)
              }, (s: ThingProperty) => s.identifier)
            }
            if (this.result !== '') {
              Text(this.result)
                .fontSize(14)
                .fontColor('#4CAF50')
                .padding(12)
            }
            if (this.error !== '') {
              Text(this.error)
                .fontSize(14)
                .fontColor('#D32F2F')
                .padding(12)
            }
          }
          .width('100%')
          .padding({ left: 16, right: 16, bottom: 16 })
        }
        .width('100%')
        .layoutWeight(1)
        .align(Alignment.Top)
      }
    }
    .width('100%')
    .height('100%')
    .backgroundColor('#F5F5F5')
  }
}
