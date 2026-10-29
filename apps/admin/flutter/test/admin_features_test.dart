import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:iot_admin/src/auth_controller.dart';
import 'package:iot_admin/src/pages/notifications_tab.dart';
import 'package:iot_admin/src/scene.dart';
import 'package:iot_admin/src/value_parse.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('AuthController reads admin role from JWT payload', () async {
    String part(Map<String, dynamic> m) =>
        base64Url.encode(utf8.encode(jsonEncode(m))).replaceAll('=', '');
    SharedPreferences.setMockInitialValues({});
    final auth = AuthController(await SharedPreferences.getInstance());
    await auth.save(
        '${part({'alg': 'HS256'})}.${part({'sub': 't1', 'role': 'admin'})}.sig');
    expect(auth.role, 'admin');
    expect(auth.isAdmin, isTrue);
    await auth.save('not-a-jwt');
    expect(auth.role, isNull);
    expect(auth.isAdmin, isFalse);
  });

  test('parseScalar keeps JSON scalars and raw strings', () {
    expect(parseScalar('true'), isTrue);
    expect(parseScalar('42'), 42);
    expect(parseScalar('3.5'), 3.5);
    expect(parseScalar('"on"'), 'on');
    expect(parseScalar('heating'), 'heating');
    expect(parseScalar(''), '');
  });

  test('Scene keeps conditions/actions and toggles enabled', () {
    final scene = Scene.fromJson({
      'id': 7,
      'name': 'away',
      'enabled': false,
      'conditions': [
        {
          'type': 'property',
          'device_id': 'd1',
          'code': 'temp',
          'operator': 'gt',
          'value': 30,
        },
      ],
      'actions': [
        {'type': 'notify', 'title': 'hot', 'message': 'temperature high'},
      ],
    });
    expect(scene.id, '7');
    expect(scene.enabled, isFalse);
    final json = scene.toJson(enabled: true);
    expect(json['enabled'], isTrue);
    expect((json['conditions'] as List).first['code'], 'temp');
    expect((json['actions'] as List).first['message'], 'temperature high');
  });

  test('AppNotification accepts alternative field names', () {
    final n = AppNotification.fromJson({
      'notification_id': 3,
      'body': 'hi',
      'read_at': '2026-10-29 10:00:00',
      'ts': 123,
    });
    expect(n.id, '3');
    expect(n.title, 'hi');
    expect(n.message, 'hi');
    expect(n.read, isTrue);
    expect(n.createdAt, '123');
  });
}
