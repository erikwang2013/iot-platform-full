import 'dart:async';

import 'package:flutter/material.dart';
import 'package:iot_shared/iot_shared.dart';
import 'package:provider/provider.dart';

import '../../l10n/app_localizations.dart';
import '../auth_controller.dart';

/// 实时告警：先拉历史记录（/api/v1/rule/alerts），再连接 WebSocket
/// 接收实时消息，新消息置顶。生产走同域 /ws 反代，本地开发直连 8084。
class AlertsPage extends StatefulWidget {
  const AlertsPage({super.key});

  @override
  State<AlertsPage> createState() => _AlertsPageState();
}

class _AlertsPageState extends State<AlertsPage> {
  static const _wsBase = String.fromEnvironment('WS_BASE');
  static const _maxItems = 100;

  final List<AlertRecord> _records = [];
  final List<AlertMessage> _live = [];
  bool _wsOk = false;
  StreamSubscription<AlertMessage>? _sub;
  Future<List<AlertRecord>>? _fetch;

  Future<List<AlertRecord>> _loadRecords() async {
    final api = context.read<ApiClient>();
    final resp = await api.get('/api/v1/rule/alerts');
    final list = resp is Map ? resp['alerts'] : resp;
    final records = parseList<AlertRecord>(list, AlertRecord.fromJson);
    _records
      ..clear()
      ..addAll(records);
    return records;
  }

  /// WS_BASE 显式指定优先；本地开发直连 rule 服务 8084，
  /// 其他环境跟随页面协议/域名走同域 /ws 反代。
  String _wsUrl(String apiBase) {
    if (_wsBase.isNotEmpty) return _wsBase;
    final base = Uri.parse(apiBase);
    if (base.host == 'localhost' || base.host == '127.0.0.1') {
      return 'ws://${base.host}:8084';
    }
    return '${base.scheme == 'https' ? 'wss' : 'ws'}://${base.authority}';
  }

  void _connect() {
    final api = context.read<ApiClient>();
    final auth = context.read<AuthController>();
    _sub?.cancel();
    _sub = alertStream(_wsUrl(api.baseUrl), auth.token ?? '')
        .listen((m) {
      if (mounted) {
        setState(() {
          _wsOk = true;
          _live.insert(0, m);
          if (_live.length > _maxItems) _live.removeLast();
        });
      }
    }, onError: (_) {
      if (mounted) setState(() => _wsOk = false);
    }, onDone: () {
      if (mounted) setState(() => _wsOk = false);
    });
  }

  @override
  void initState() {
    super.initState();
    _fetch = _loadRecords();
    _connect();
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  String _time(int ts) {
    final t = DateTime.fromMillisecondsSinceEpoch(ts).toLocal();
    String two(int n) => n.toString().padLeft(2, '0');
    return '${two(t.hour)}:${two(t.minute)}:${two(t.second)}';
  }

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context)!;
    return Scaffold(
      appBar: AppBar(
        title: Text(l10n.alertCenter),
        bottom: PreferredSize(
          preferredSize: const Size.fromHeight(36),
          child: Row(
            children: [
              const SizedBox(width: 16),
              Icon(
                _wsOk ? Icons.wifi : Icons.wifi_off,
                size: 16,
                color: _wsOk ? Colors.green : Colors.redAccent,
              ),
              const SizedBox(width: 8),
              Text(_wsOk ? l10n.wsConnected : l10n.wsDisconnected,
                  style: Theme.of(context).textTheme.bodySmall),
            ],
          ),
        ),
      ),
      body: FutureBuilder<List<AlertRecord>>(
        future: _fetch,
        builder: (context, snap) {
          final records = snap.data ?? _records;
          if (_live.isEmpty && records.isEmpty) {
            if (snap.connectionState != ConnectionState.done) {
              return const Center(child: CircularProgressIndicator());
            }
            if (snap.hasError) {
              return Center(child: Text('${snap.error}'));
            }
            return Center(child: Text(l10n.alertNoAlerts));
          }
          return ListView.builder(
            itemCount: _live.length + records.length,
            itemBuilder: (context, i) {
              if (i < _live.length) {
                final m = _live[i];
                return ListTile(
                  leading: const Icon(Icons.warning_amber,
                      color: Colors.redAccent),
                  title: Text(m.summary),
                  subtitle: Text('${m.deviceId} · ${_time(m.ts)}'),
                );
              }
              final r = records[i - _live.length];
              return ListTile(
                leading: Icon(
                  r.acknowledged ? Icons.check_circle_outline : Icons.error_outline,
                  color: r.acknowledged ? Colors.grey : Colors.orange,
                ),
                title: Text(
                    '${r.code} ${r.operator} ${r.threshold} → ${r.value}'),
                subtitle: Text(
                    '${r.deviceId} · ${r.acknowledged ? l10n.alertAcknowledged : l10n.alertActive} · ${r.createdAt}'),
              );
            },
          );
        },
      ),
    );
  }
}
