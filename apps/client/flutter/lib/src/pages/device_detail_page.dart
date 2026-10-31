import 'dart:async';

import 'package:flutter/material.dart';
import 'package:iot_shared/iot_shared.dart';
import 'package:provider/provider.dart';

import '../../l10n/app_localizations.dart';
import '../widgets/thing_model_panel.dart';
import 'history_page.dart';

/// 设备详情：基本信息 + 物模型动态渲染 + 指令下发 + 历史曲线入口。
/// 在线状态与最多 6 个可读属性的最新值每 5 秒轮询（REST，非 WebSocket）。
class DeviceDetailPage extends StatefulWidget {
  const DeviceDetailPage({super.key, required this.device, this.groupName});

  final Device device;
  final String? groupName;

  @override
  State<DeviceDetailPage> createState() => _DeviceDetailPageState();
}

class _DeviceDetailPageState extends State<DeviceDetailPage> {
  late Future<ThingModel> _modelFuture;
  ThingModel? _model;
  Timer? _timer;
  bool _polling = false;
  late bool _online = widget.device.online;
  late String _status = widget.device.status;
  Map<String, String> _latest = const {};

  Future<ThingModel> _fetchModel() async {
    final api = context.read<ApiClient>();
    final resp = await api.get('/api/v1/models/things/${widget.device.id}');
    final model = resp is Map && (resp['properties'] != null ||
            resp['events'] != null ||
            resp['services'] != null)
        ? ThingModel.fromJson(Map<String, dynamic>.from(resp))
        : ThingModel.empty();
    _model = model;
    return model;
  }

  Future<void> _sendCommand(String code, dynamic value) async {
    final api = context.read<ApiClient>();
    await api.post(
      '/api/v1/access/devices/${widget.device.id}/command',
      body: {'code': code, 'value': value},
    );
  }

  String _fmtNum(num v) =>
      v % 1 == 0 ? v.toInt().toString() : v.toStringAsFixed(2);

  Future<void> _poll() async {
    if (_polling) return;
    _polling = true;
    final api = context.read<ApiClient>();
    try {
      try {
        final resp = await api.get('/api/v1/devices');
        final devices = parseList<Device>(
            resp is Map ? resp['devices'] : resp, Device.fromJson);
        for (final d in devices) {
          if (d.id != widget.device.id) continue;
          if (mounted) {
            setState(() {
              _online = d.online;
              _status = d.status;
            });
          }
          break;
        }
      } catch (_) {
        // 在线状态轮询失败时保留上次结果
      }
      final model = _model;
      if (model == null) return;
      final props = model.properties
          .where((p) => p.rw != 'w' && p.type == 'number')
          .take(6)
          .toList();
      if (props.isEmpty) return;
      final end = DateTime.now().millisecondsSinceEpoch;
      final start = end - const Duration(hours: 1).inMilliseconds;
      final entries = await Future.wait(
        props.map<Future<MapEntry<String, String>?>>((p) async {
          try {
            final resp = await api.get('/api/v1/data/history', query: {
              'device_id': widget.device.id,
              'code': p.identifier,
              'start': '$start',
              'end': '$end',
              'agg': 'last',
              'interval': '1h',
              'limit': '1',
            });
            final points = parseList<HistoryPoint>(
                resp is Map ? resp['points'] : null, HistoryPoint.fromJson);
            return points.isEmpty
                ? null
                : MapEntry(p.identifier, _fmtNum(points.last.value));
          } catch (_) {
            // 单个属性查询失败不影响其他属性
            return null;
          }
        }),
      );
      if (!mounted) return;
      setState(() {
        _latest = {
          for (final e in entries)
            if (e != null) e.key: e.value,
        };
      });
    } finally {
      _polling = false;
    }
  }

  String _label(String code) {
    for (final p in _model?.properties ?? const <ThingProperty>[]) {
      if (p.identifier == code && p.name.isNotEmpty) return p.name;
    }
    return code;
  }

  @override
  void initState() {
    super.initState();
    _modelFuture = _fetchModel();
    _poll();
    _timer = Timer.periodic(const Duration(seconds: 5), (_) => _poll());
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context)!;
    final d = widget.device;
    return Scaffold(
      appBar: AppBar(title: Text(d.name.isEmpty ? d.id : d.name)),
      body: Column(
        children: [
          ListTile(
            leading: Icon(
              _online ? Icons.circle : Icons.circle_outlined,
              color: _online ? Colors.green : Colors.grey,
            ),
            title: Text(l10n.deviceStatus),
            isThreeLine: true,
            subtitle: Text(
              '$_status · ${_online ? l10n.deviceOnline : l10n.deviceOffline}'
              ' · ${d.vendor} · ${d.id}\n${l10n.adminPolling}',
            ),
          ),
          if (widget.groupName != null && widget.groupName!.isNotEmpty)
            ListTile(
              dense: true,
              leading: const Icon(Icons.folder_outlined),
              title: Text(l10n.adminFilterGroup),
              subtitle: Text(widget.groupName!),
            ),
          if (_latest.isNotEmpty)
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(l10n.adminLatestValues,
                      style: Theme.of(context).textTheme.titleSmall),
                  const SizedBox(height: 4),
                  Wrap(
                    spacing: 8,
                    runSpacing: 4,
                    children: [
                      for (final e in _latest.entries)
                        Chip(
                          visualDensity: VisualDensity.compact,
                          label: Text('${_label(e.key)} ${e.value}'),
                        ),
                    ],
                  ),
                ],
              ),
            ),
          const Divider(height: 1),
          Expanded(
            child: FutureBuilder<ThingModel>(
              future: _modelFuture,
              builder: (context, snap) {
                if (snap.connectionState != ConnectionState.done) {
                  return const Center(child: CircularProgressIndicator());
                }
                if (snap.hasError) {
                  return Center(
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Padding(
                          padding: const EdgeInsets.all(24),
                          child: Text('${snap.error}',
                              textAlign: TextAlign.center),
                        ),
                        FilledButton.tonal(
                          onPressed: () => setState(() {
                            _modelFuture = _fetchModel();
                          }),
                          child: Text(l10n.commonRetry),
                        ),
                      ],
                    ),
                  );
                }
                final model = snap.data ?? ThingModel.empty();
                if (model.properties.isEmpty &&
                    model.events.isEmpty &&
                    model.services.isEmpty) {
                  return Center(child: Text(l10n.commonEmpty));
                }
                return Column(
                  children: [
                    ListTile(
                      dense: true,
                      leading: const Icon(Icons.show_chart),
                      title: Text(l10n.navHistory),
                      trailing: const Icon(Icons.chevron_right),
                      onTap: () => Navigator.of(context).push(
                        MaterialPageRoute(
                          builder: (_) => DeviceHistoryPage(
                            deviceId: d.id,
                            properties: model.properties,
                          ),
                        ),
                      ),
                    ),
                    const Divider(height: 1),
                    Expanded(
                      child: ThingModelPanel(
                        model: model,
                        onCommand: _sendCommand,
                      ),
                    ),
                  ],
                );
              },
            ),
          ),
        ],
      ),
    );
  }
}
