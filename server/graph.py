import matplotlib.pyplot as plt
from matplotlib.dates import ConciseDateFormatter, AutoDateLocator
from mpl_toolkits.axes_grid1 import host_subplot
from mpl_toolkits import axisartist
import dateutil
import csv

def read_data(file):
    with open(file, newline='') as csvfile:
        reader = csv.DictReader(csvfile, fieldnames=["time", "temperature", "humidity", "pressure", "battery"])
        data = []
        for row in reader:
            row["time"] = dateutil.parser.parse(row["time"]).astimezone(tz=None) 
            row["temperature"] = float(row["temperature"])
            row["humidity"] = float(row["humidity"])
            row["pressure"] = float(row["pressure"])
            row["battery"] = float(row["battery"])
            data.append(row)
        return data

data = read_data("readings.csv")
# make shure values are ordered in the order they were recorded
data.sort(key=lambda x: x["time"])

host = host_subplot(111, axes_class=axisartist.Axes)
plt.subplots_adjust(right=0.75)

par1 = host.twinx()
par2 = host.twinx()
par3 = host.twinx()

par2.axis["right"] = par2.new_fixed_axis(loc="right", offset=(60, 0))
par3.axis["right"] = par3.new_fixed_axis(loc="right", offset=(60*2, 0))

par1.axis["right"].toggle(all=True)
par2.axis["right"].toggle(all=True)
par3.axis["right"].toggle(all=True)

xdata = [row["time"] for row in data]
p, = host.plot(xdata, [row["battery"] for row in data], label="Battery")
p1, = par1.plot(xdata, [row["temperature"] * 1.8 + 32.0 for row in data], label="Temperature")
p2, = par2.plot(xdata, [row["humidity"] for row in data], label="Humidity")
p3, = par3.plot(xdata, [row["pressure"] for row in data], label="Pressure")

# host.set_xlim(0, 2)
# host.set_ylim(0, 2)
# par1.set_ylim(0, 4)
# par2.set_ylim(1, 65)

host.set_xlabel("Time")
host.grid()
host.xaxis.set_major_formatter(ConciseDateFormatter(AutoDateLocator(tz="EST"), tz="EST"))

host.set_ylabel("Battery")
par1.set_ylabel("Temperature")
par2.set_ylabel("Humidity")
par3.set_ylabel("Pressure")

host.legend()

host.axis["left"].label.set_color(p.get_color())
par1.axis["right"].label.set_color(p1.get_color())
par2.axis["right"].label.set_color(p2.get_color())
par3.axis["right"].label.set_color(p3.get_color())

plt.show()
#-----------------------------------
# fig = plt.figure()
# color = 'tab:red'
# ax = fig.add_subplot(111, label="temperature")
# ax.plot([row["time"] for row in data], [row["temperature"] * 1.8 + 32.0 for row in data], color=color)
# ax.set(xlabel='time', title='Temperature')
# ax.set_ylabel("temp (F)", color=color)
# ax.tick_params(axis='y', labelcolor=color)
#
# ax.grid()
# ax.xaxis.set_major_formatter(ConciseDateFormatter(AutoDateLocator(tz="EST"), tz="EST"))
#
# color = 'tab:blue'
# ax1 = ax.twinx()
# ax1.plot([row["time"] for row in data], [row["humidity"] for row in data], color=color)
# ax1.yaxis.tick_right()
# ax1.set_ylabel("humidity (%RH)", color=color)
# ax1.tick_params(axis='y', labelcolor=color)
#
# fig.tight_layout()
# #fig.autofmt_xdate()
# plt.show()

