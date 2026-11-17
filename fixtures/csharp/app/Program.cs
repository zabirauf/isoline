using System;
using System.Collections.Generic;
using Json = System.Text.Json.JsonSerializer;

namespace Demo.App
{
    /// <summary>Entry point.</summary>
    public class Program
    {
        public string Name { get; set; }

        public static void Main(string[] args)
        {
            var list = new List<int>();
            Console.WriteLine(Json.Serialize(list));
        }
    }

    internal struct Point
    {
        public int X;
    }

    public interface IShape
    {
        double Area();
    }
}
