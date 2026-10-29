import 'package:flutter_test/flutter_test.dart';
import 'package:iot_client/src/pages/scenes_page.dart';
import 'package:iot_client/src/scene.dart';

void main() {
  test('Scene parses backend JSON, enabled defaults to true', () {
    final s = Scene.fromJson({
      'id': '7',
      'name': '回家',
      'enabled': false,
      'conditions': [
        {
          'type': 'property',
          'device_id': '12',
          'code': 'temp',
          'operator': '>',
          'value': 30,
        },
      ],
      'actions': [
        {'type': 'notify', 'title': 't', 'message': 'm'},
      ],
    });
    expect(s.id, '7');
    expect(s.enabled, isFalse);
    expect(s.conditions.single['code'], 'temp');
    expect(s.actions.single['type'], 'notify');

    final defaults = Scene.fromJson({'id': '8', 'name': 'x'});
    expect(defaults.enabled, isTrue);
    expect(defaults.conditions, isEmpty);
    expect(defaults.actions, isEmpty);
  });

  test('AppNotification maps read_at and falls back message fields', () {
    final unread =
        AppNotification.fromJson({'id': '1', 'title': 'T', 'message': 'M'});
    expect(unread.read, isFalse);
    expect(unread.message, 'M');

    final read = AppNotification.fromJson({
      'id': '2',
      'title': 'T',
      'message': 'M',
      'read_at': '2026-01-01',
    });
    expect(read.read, isTrue);

    final fallback = AppNotification.fromJson({'id': '3', 'body': 'B'});
    expect(fallback.title, 'B');
  });
}
