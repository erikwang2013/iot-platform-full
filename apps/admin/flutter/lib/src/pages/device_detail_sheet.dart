import 'dart:async';

import 'package:flutter/material.dart';
import 'package:iot_shared/iot_shared.dart';
import 'package:provider/provider.dart';

import '../../l10n/app_localizations.dart';
import '../value_parse.dart';

/// 设备详情弹层：物模型 + 指令下发 + 最近告警，打开期间每 5 秒轮询。
class DeviceDetailSheet extends StatefulWidget {
  const DeviceDetailSheet({super.key, required this.device});

  final Device device;

  @override
  State<DeviceDetailSheet> createState() => DeviceDetailSheetState();
}

class DeviceDetailSheetState extends State<DeviceDetailSheet> {
  static const _maxHistoryProps = 6;

  late final ApiClient _api = context.read<ApiClient>();
  final _value = TextEditingController();
  Timer? _timer;
  bool _busy = false;
  late String _status = widget.device.status;
  Map<String, dynamic>? _model;
  List<AlertRecord> _alerts = const [];
  final Map<String, num> _latest = {};
  String? _code;

  @override
  void initState() {
    super.initState();
    _loadModel();
    _loadAlerts();
    _tick();
    _timer = Timer.periodic(const Duration(seconds: 5), (_) => _tick());
  }

  @override
  void dispose() {
    _timer?.cancel();
    _value.dispose();
    super.dispose();
  }

  List<Map<String, dynamic>> _section(String key) {
    final v = _model?[key];
    return v is List ? v.whereType<Map<String, dynamic>>().toList() : const [];
  }

  Future<void> _loadModel() async {
    try {
      final resp = await _api.get('/api/v1/models/things/${widget.device.id}');
      if (!mounted) return;
      setState(() => _model = resp is Map<String, dynamic> ? resp : null);
      _tick();
    } catch (_) {}
  }

  Future<void> _loadAlerts() async {
    try {
      final resp = await _api.get('/api/v1/rule/alerts');
      final list = resp is Map ? resp['alerts'] : resp;
      final mine = parseList<AlertRecord>(list, AlertRecord.fromJson)
          .where((a) => a.deviceId == widget.device.id)
          .take(5)
          .toList();
      if (mounted) setState(() => _alerts = mine);
    } catch (_) {}
  }

  /// 状态 + 最新值轮询；每 5 秒一次，上一轮未结束时跳过。
  Future<void> _tick() async {
    if (_busy) return;
    _busy = true;
    final id = widget.device.id;
    try {
      final resp = await _api.get('/api/v1/devices');
      final list = resp is Map ? resp['devices'] : resp;
      Device? fresh;
      for (final d in parseList<Device>(list, Device.fromJson)) {
        if (d.id == id) {
          fresh = d;
          break;
        }
      }
      if (fresh != null && mounted && fresh.status != _status) {
        final status = fresh.status;
        setState(() => _status = status);
      }
    } catch (_) {}

    final props = _section('properties')
        .where((p) => '${p['rw'] ?? 'rw'}' != 'w')
        .take(_maxHistoryProps);
    final now = DateTime.now().millisecondsSinceEpoch;
    for (final p in props) {
      final code = '${p['identifier'] ?? p['id'] ?? ''}';
      if (code.isEmpty) continue;
      try {
        final resp = await _api.get('/api/v1/data/history', query: {
          'device_id': id,
          'code': code,
          'start': '${now - 24 * 3600 * 1000}',
          'end': '$now',
          'agg': 'last',
          'interval': '1h',
          'limit': '24',
        });
        final points = resp is Map ? resp['points'] : null;
        final list = parseList<HistoryPoint>(points, HistoryPoint.fromJson);
        if (list.isNotEmpty && mounted) {
          setState(() => _latest[code] = list.last.value);
        }
      } catch (_) {
        // 单属性历史不可用不影响其余展示
      }
    }
    _busy = false;
  }

  Future<void> _sendCommand() async {
    final l10n = AppLocalizations.of(context)!;
    final code = _code;
    if (code == null || code.isEmpty) return;
    final props = _section('properties');
    var value = parseScalar(_value.text);
    for (final p in props) {
      if ('${p['identifier'] ?? p['id']}' != code) continue;
      final type = '${p['data_type'] ?? ''}';
      if (type.contains('bool') && value is! bool) {
        value = _value.text.trim() == 'true';
      } else if (type.contains('num') ||
          type.contains('float') ||
          type.contains('int') ||
          type.contains('double')) {
        value = num.tryParse(_value.text.trim()) ?? value;
      } else if (type.contains('string') && value is! String) {
        value = _value.text;
      }
      break;
    }
    try {
      final resp = await _api.post(
          '/api/v1/access/devices/${widget.device.id}/command',
          body: {'code': code, 'value': value});
      if (!mounted) return;
      final queued = resp is Map && resp['queued'] == true;
      final channel = resp is Map ? resp['channel'] : null;
      _toast(queued
          ? l10n.adminCommandQueued
          : l10n.adminResultValue('${channel ?? 'ok'}'));
      _tick();
    } catch (e) {
      if (mounted) _toast('$e');
    }
  }

  void _toast(String msg) {
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(msg)));
  }

  Widget _kv(String k, String v) => Padding(
        padding: const EdgeInsets.symmetric(vertical: 4),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
                width: 120,
                child: Text(k, style: Theme.of(context).textTheme.bodySmall)),
            Expanded(child: Text(v)),
          ],
        ),
      );

  Widget _title(String text) => Padding(
        padding: const EdgeInsets.only(top: 12, bottom: 4),
        child: Text(text, style: Theme.of(context).textTheme.titleSmall),
      );

  String _propValue(Map<String, dynamic> p, AppLocalizations l10n) {
    final unit = '${p['unit'] ?? ''}';
    final latest = _latest['${p['identifier'] ?? p['id']}'] ?? '—';
    return '${p['data_type'] ?? p['type'] ?? ''}'
        '${unit.isEmpty ? '' : ' $unit'} · ${p['rw'] ?? ''}'
        ' · ${l10n.adminLatestValues}: $latest';
  }

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context)!;
    final d = widget.device;
    final props = _section('properties');
    final events = _section('events');
    final services = _section('services');
    const shown = {'id', 'device_id', 'name', 'vendor', 'status', 'group'};
    return ListView(
      padding: const EdgeInsets.all(16),
      children: [
        Row(
          children: [
            Expanded(
              child: Text(d.name.isEmpty ? d.id : d.name,
                  style: Theme.of(context).textTheme.titleMedium),
            ),
            IconButton(
              icon: const Icon(Icons.close),
              onPressed: () => Navigator.pop(context),
            ),
          ],
        ),
        Text(l10n.adminPolling,
            style: Theme.of(context).textTheme.bodySmall),
        const SizedBox(height: 8),
        _kv(l10n.deviceName, d.name),
        _kv('ID', d.id),
        _kv(
            l10n.deviceStatus,
            '$_status (${d.online ? l10n.deviceOnline : l10n.deviceOffline})'),
        _kv('vendor', d.vendor),
        if (d.group.isNotEmpty) _kv('group', d.group),
        for (final e in d.extra.entries
            .where((e) => !shown.contains(e.key))
            .take(12))
          _kv(e.key, '${e.value}'),
        const Divider(),
        _title(l10n.adminThingModel),
        if (props.isEmpty && events.isEmpty && services.isEmpty)
          Text(l10n.commonEmpty)
        else ...[
          if (props.isNotEmpty) Text(l10n.modelProperty),
          for (final p in props)
            _kv(
              '${p['name'] ?? p['identifier']} (${p['identifier']})',
              _propValue(p, l10n),
            ),
          if (events.isNotEmpty) Text(l10n.modelEvent),
          for (final e in events)
            _kv('${e['name'] ?? e['identifier']}', '${e['identifier']}'),
          if (services.isNotEmpty) Text(l10n.modelService),
          for (final s in services)
            _kv('${s['name'] ?? s['identifier']}', '${s['identifier']}'),
        ],
        if (props.isNotEmpty) ...[
          const Divider(),
          _title(l10n.adminCommandTitle),
          DropdownButtonFormField<String>(
            initialValue: _code,
            isDense: true,
            decoration: InputDecoration(
                labelText: l10n.adminCommandCode,
                border: const OutlineInputBorder()),
            items: [
              for (final p in props)
                DropdownMenuItem(
                  value: '${p['identifier'] ?? p['id']}',
                  child: Text(
                      '${p['name'] ?? p['identifier']} (${p['identifier']})'),
                ),
            ],
            onChanged: (v) => setState(() => _code = v),
          ),
          const SizedBox(height: 8),
          TextField(
            controller: _value,
            decoration: InputDecoration(
                labelText: l10n.adminCommandValue,
                border: const OutlineInputBorder()),
          ),
          Align(
            alignment: Alignment.centerRight,
            child: FilledButton.icon(
              onPressed: _code == null ? null : _sendCommand,
              icon: const Icon(Icons.send),
              label: Text(l10n.adminCommandSend),
            ),
          ),
        ],
        const Divider(),
        _title(l10n.adminRecentAlerts),
        if (_alerts.isEmpty)
          Text(l10n.alertNoAlerts)
        else
          for (final a in _alerts)
            ListTile(
              dense: true,
              contentPadding: EdgeInsets.zero,
              title: Text(
                  '${a.code} ${a.operator} ${a.threshold} → ${a.value}'),
              subtitle: Text('${a.status} · ${a.createdAt}'),
            ),
      ],
    );
  }
}
