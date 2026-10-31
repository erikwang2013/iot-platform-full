import 'package:flutter/material.dart';
import 'package:iot_shared/iot_shared.dart';
import 'package:provider/provider.dart';

import '../../l10n/app_localizations.dart';

/// 首页概览：/api/v1/devices/stats 设备用量 + 厂商分布。
class DeviceStats {
  DeviceStats({
    this.total = 0,
    this.online = 0,
    this.offline = 0,
    this.vendors = const [],
  });

  final int total;
  final int online;
  final int offline;
  final List<({String vendor, int count})> vendors;

  factory DeviceStats.fromJson(Map<String, dynamic> j) {
    int n(String key) => j[key] is num ? (j[key] as num).toInt() : 0;
    final vendors = j['vendors'];
    return DeviceStats(
      total: n('total'),
      online: n('online'),
      offline: n('offline'),
      vendors: vendors is List
          ? [
              for (final v in vendors.whereType<Map>())
                (
                  vendor: '${v['vendor'] ?? ''}',
                  count: v['count'] is num ? (v['count'] as num).toInt() : 0,
                ),
            ]
          : const [],
    );
  }
}

class DashboardPage extends StatefulWidget {
  const DashboardPage({super.key});

  @override
  State<DashboardPage> createState() => _DashboardPageState();
}

class _DashboardPageState extends State<DashboardPage> {
  late Future<DeviceStats> _future;

  Future<DeviceStats> _load() async {
    final api = context.read<ApiClient>();
    final resp = await api.get('/api/v1/devices/stats');
    if (resp is Map) return DeviceStats.fromJson(Map<String, dynamic>.from(resp));
    throw ApiListError('unexpected response shape: ${resp.runtimeType}');
  }

  @override
  void initState() {
    super.initState();
    _future = _load();
  }

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context)!;
    return Scaffold(
      appBar: AppBar(
        title: Text(l10n.navDashboard),
        actions: [
          IconButton(
            onPressed: () => setState(() => _future = _load()),
            icon: const Icon(Icons.refresh),
            tooltip: l10n.commonRetry,
          ),
        ],
      ),
      body: FutureBuilder<DeviceStats>(
        future: _future,
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
                    child: Text('${snap.error}', textAlign: TextAlign.center),
                  ),
                  OutlinedButton.icon(
                    onPressed: () => setState(() => _future = _load()),
                    icon: const Icon(Icons.refresh),
                    label: Text(l10n.commonRetry),
                  ),
                ],
              ),
            );
          }
          final stats = snap.data ?? DeviceStats();
          return ListView(
            padding: const EdgeInsets.all(16),
            children: [
              Row(
                children: [
                  Expanded(
                    child: _StatCard(
                      label: l10n.statTotalDevices,
                      value: '${stats.total}',
                      icon: Icons.devices_outlined,
                    ),
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: _StatCard(
                      label: l10n.statOnlineDevices,
                      value: '${stats.online}',
                      icon: Icons.circle,
                      color: Colors.green,
                    ),
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: _StatCard(
                      label: l10n.statOfflineDevices,
                      value: '${stats.offline}',
                      icon: Icons.circle_outlined,
                      color: Colors.grey,
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 16),
              Text(l10n.statVendorDist,
                  style: Theme.of(context).textTheme.titleSmall),
              const SizedBox(height: 8),
              Card(
                child: Column(
                  children: [
                    if (stats.vendors.isEmpty)
                      ListTile(dense: true, title: Text(l10n.commonEmpty)),
                    for (final v in stats.vendors)
                      ListTile(
                        dense: true,
                        title: Text(v.vendor.isEmpty ? '—' : v.vendor),
                        trailing: Text('${v.count}'),
                      ),
                  ],
                ),
              ),
            ],
          );
        },
      ),
    );
  }
}

class _StatCard extends StatelessWidget {
  const _StatCard({
    required this.label,
    required this.value,
    required this.icon,
    this.color,
  });

  final String label;
  final String value;
  final IconData icon;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Icon(icon, color: color ?? Theme.of(context).colorScheme.primary),
            const SizedBox(height: 8),
            Text(value, style: Theme.of(context).textTheme.headlineMedium),
            Text(label, style: Theme.of(context).textTheme.bodySmall),
          ],
        ),
      ),
    );
  }
}
