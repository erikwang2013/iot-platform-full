import 'package:flutter/material.dart';
import 'package:iot_shared/iot_shared.dart';
import 'package:provider/provider.dart';

import '../../l10n/app_localizations.dart';
import 'device_detail_page.dart';

/// 设备分组（/api/v1/devices/groups 行）。
class DeviceGroup {
  DeviceGroup({required this.id, this.name = '', this.memberCount = 0});

  final String id;
  final String name;
  final int memberCount;

  factory DeviceGroup.fromJson(Map<String, dynamic> j) => DeviceGroup(
        id: '${j['id'] ?? ''}',
        name: '${j['name'] ?? ''}',
        memberCount:
            j['member_count'] is num ? (j['member_count'] as num).toInt() : 0,
      );
}

/// 客户端设备列表：分组筛选 + 本地搜索，点击进详情。
class DeviceListPage extends StatefulWidget {
  const DeviceListPage({super.key});

  @override
  State<DeviceListPage> createState() => _DeviceListPageState();
}

class _DeviceListPageState extends State<DeviceListPage> {
  final _query = TextEditingController();
  String _filter = '';
  String? _groupId;
  List<DeviceGroup> _groups = [];
  late Future<List<DeviceGroup>> _groupsFuture;

  Future<List<DeviceGroup>> _loadGroups() async {
    final api = context.read<ApiClient>();
    final resp = await api.get('/api/v1/devices/groups');
    final list = resp is Map ? resp['groups'] : resp;
    final groups = parseList<DeviceGroup>(list, DeviceGroup.fromJson);
    _groups = groups;
    return groups;
  }

  Future<List<Device>> _load() async {
    final api = context.read<ApiClient>();
    final resp = await api.get(
      '/api/v1/devices',
      query: _groupId == null ? null : {'group_id': _groupId!},
    );
    final list = resp is Map ? resp['devices'] : resp;
    return parseList<Device>(list, Device.fromJson);
  }

  String? get _groupName {
    for (final g in _groups) {
      if (g.id == _groupId) return g.name;
    }
    return null;
  }

  @override
  void initState() {
    super.initState();
    _groupsFuture = _loadGroups();
  }

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context)!;
    return Scaffold(
      appBar: AppBar(
        title: Text(l10n.deviceMyDevices),
        bottom: PreferredSize(
          preferredSize: const Size.fromHeight(64),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(16, 0, 16, 12),
            child: TextField(
              controller: _query,
              onChanged: (v) => setState(() => _filter = v.trim()),
              decoration: InputDecoration(
                hintText: l10n.commonSearch,
                prefixIcon: const Icon(Icons.search),
                border: const OutlineInputBorder(),
                isDense: true,
              ),
            ),
          ),
        ),
      ),
      body: Column(
        children: [
          FutureBuilder<List<DeviceGroup>>(
            future: _groupsFuture,
            builder: (context, snap) {
              final groups = snap.data ?? _groups;
              if (groups.isEmpty) return const SizedBox.shrink();
              return SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
                child: Row(
                  children: [
                    ChoiceChip(
                      label: Text(l10n.adminFilterAll),
                      selected: _groupId == null,
                      onSelected: (_) => setState(() => _groupId = null),
                    ),
                    for (final g in groups)
                      Padding(
                        padding: const EdgeInsetsDirectional.only(start: 8),
                        child: ChoiceChip(
                          label: Text(g.memberCount > 0
                              ? '${g.name} (${g.memberCount})'
                              : g.name),
                          selected: _groupId == g.id,
                          onSelected: (_) => setState(() => _groupId = g.id),
                        ),
                      ),
                  ],
                ),
              );
            },
          ),
          Expanded(
            child: ApiList<Device>(
              key: ValueKey(_groupId),
              load: _load,
              emptyText: l10n.commonEmpty,
              builder: (context, devices) {
                final filtered = _filter.isEmpty
                    ? devices
                    : devices
                        .where((d) =>
                            d.name.contains(_filter) || d.id.contains(_filter))
                        .toList();
                if (filtered.isEmpty) {
                  return Center(child: Text(l10n.commonEmpty));
                }
                return ListView.builder(
                  itemCount: filtered.length,
                  itemBuilder: (context, i) {
                    final d = filtered[i];
                    return ListTile(
                      leading: Icon(
                        d.online ? Icons.circle : Icons.circle_outlined,
                        color: d.online ? Colors.green : Colors.grey,
                      ),
                      title: Text(d.name.isEmpty ? d.id : d.name),
                      subtitle: Text('${d.vendor} · ${d.id}'),
                      trailing: const Icon(Icons.chevron_right),
                      onTap: () => Navigator.of(context).push(
                        MaterialPageRoute(
                          builder: (_) => DeviceDetailPage(
                            device: d,
                            groupName: _groupName,
                          ),
                        ),
                      ),
                    );
                  },
                );
              },
            ),
          ),
        ],
      ),
    );
  }
}
