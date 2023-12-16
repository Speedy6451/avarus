ipaddr = "68.46.126.104:48228"
--ipaddr = "localhost:48228"

local idfile = fs.open("id", "r")

local id = nil
local command = nil

if not idfile then
    local fuel = turtle.getFuelLevel()
    local stdin = io.input()
    print("Direction (North, South, East, West):")
    local direction = stdin:read("l")
    print("X:")
    local x = tonumber(stdin:read("l"))
    print("Y:")
    local x = tonumber(stdin:read("l"))
    print("Z:")
    local x = tonumber(stdin:read("l"))
    local y = tonumber(stdin:read("l"))
    local z = tonumber(stdin:read("l"))

    local info = {
        fuel = fuel,
        position = {x, y, z},
        facing = direction,
    }
    -- TODO: get from boot floppy
    local turtleinfo = http.post(
        "http://" .. ipaddr .. "/turtle/new",
        textutils.serializeJSON(info),
        { ["Content-Type"] = "application/json" }
    )
    local response = textutils.unserialiseJSON(turtleinfo.readAll())

    idfile = fs.open("id", "w")
    idfile.write(response.id)
    idfile.close()
    os.setComputerLabel(response.name)
    id = response.id
    command = response.command
else
    id = idfile.readAll()
    idfile.close()
end

print(command)
